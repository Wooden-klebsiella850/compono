//! Rendu GPU des overlays : D3D11 + DirectComposition + Direct2D.
//!
//! L'initialisation GPU est paresseuse : elle n'a lieu qu'à la première
//! apparition de la grille, pour garder un démarrage du tray instantané.

use std::mem::ManuallyDrop;

use windows::core::{Interface, Result};
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_BUFFER_PRECISION_8BPC_UNORM, D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT,
    D2D1_COLOR_SPACE_SRGB, D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_EXTEND_MODE_CLAMP,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_ROUNDED_RECT,
    D2D1CreateFactory, ID2D1Bitmap1, ID2D1ColorContext, ID2D1Device, ID2D1DeviceContext,
    ID2D1Factory1, ID2D1GradientStopCollection1, ID2D1LinearGradientBrush, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, ID3D11Device,
    ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactory2,
    IDXGISurface, IDXGISwapChain1,
};
use windows_numerics::Vector2;

use crate::grid::{Grid, Rect};
use crate::monitors::ScreenEdge;

/// Couleur d'accent (vert).
const ACCENT: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.239,
    g: 0.862,
    b: 0.517,
    a: 1.0,
};

/// Opacités du rendu.
const GRID_LINE_ALPHA: f32 = 0.07;
const GRID_DOT_ALPHA: f32 = 0.15;
const SELECTION_FILL_ALPHA: f32 = 0.14;
const SELECTION_BORDER_ALPHA: f32 = 0.70;
const SELECTION_RADIUS: f32 = 6.0;
const SELECTION_BORDER_WIDTH: f32 = 2.0;

/// État dessiné par un overlay : halo de bord et rectangle de sélection.
#[derive(Debug, Clone, Copy)]
pub struct OverlayState {
    pub halo: Option<HaloState>,
    pub selection: Option<Rect>,
}

/// Halo dégradé sur un bord, vers l'intérieur de l'écran.
#[derive(Debug, Clone, Copy)]
pub struct HaloState {
    pub edge: ScreenEdge,
    /// Épaisseur du halo en pixels.
    pub thickness: f32,
}

impl Default for OverlayState {
    fn default() -> Self {
        Self {
            halo: None,
            selection: None,
        }
    }
}

/// Ressources GPU partagées entre tous les overlays.
pub struct Gfx {
    device: ID3D11Device,
    _context: ID3D11DeviceContext,
    // Gardés vivants : les objets enfants n'en tiennent pas forcément de référence.
    #[allow(dead_code)]
    dxgi_device: IDXGIDevice,
    dcomp: IDCompositionDevice,
    #[allow(dead_code)]
    d2d_factory: ID2D1Factory1,
    d2d_device: ID2D1Device,
}

impl Gfx {
    pub fn new() -> Result<Gfx> {
        unsafe {
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE(std::ptr::null_mut()),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
            let device = device.expect("device D3D11");
            let context = context.expect("contexte D3D11");
            let dxgi_device: IDXGIDevice = device.cast()?;
            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi_device)?;
            let d2d_factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d_device: ID2D1Device = d2d_factory.CreateDevice(&dxgi_device)?;
            Ok(Gfx {
                device,
                _context: context,
                dxgi_device,
                dcomp,
                d2d_factory,
                d2d_device,
            })
        }
    }
}

/// Rendu d'une fenêtre overlay : swapchain, contexte D2D, cible et arbre DComp.
pub struct Renderer {
    swapchain: IDXGISwapChain1,
    dcomp: IDCompositionDevice,
    _dcomp_target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    d2d_context: ID2D1DeviceContext,
    _target: ID2D1Bitmap1,
}

impl Renderer {
    pub fn new(gfx: &Gfx, hwnd: HWND, width: u32, height: u32) -> Result<Renderer> {
        unsafe {
            let factory: IDXGIFactory2 = CreateDXGIFactory2(Default::default())?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width,
                Height: height,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                Flags: 0,
            };
            let swapchain: IDXGISwapChain1 =
                factory.CreateSwapChainForComposition(&gfx.device, &desc, None)?;
            let visual: IDCompositionVisual = gfx.dcomp.CreateVisual()?;
            let target: IDCompositionTarget = gfx.dcomp.CreateTargetForHwnd(hwnd, true)?;
            visual.SetContent(&swapchain)?;
            target.SetRoot(&visual)?;
            gfx.dcomp.Commit()?;

            let d2d_context: ID2D1DeviceContext =
                gfx.d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let surface: IDXGISurface = swapchain.GetBuffer::<ID3D11Texture2D>(0)?.cast()?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 0.0,
                dpiY: 0.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
                colorContext: ManuallyDrop::new(None::<ID2D1ColorContext>),
            };
            let target_bitmap: ID2D1Bitmap1 =
                d2d_context.CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
            d2d_context.SetTarget(&target_bitmap);

            Ok(Renderer {
                swapchain,
                dcomp: gfx.dcomp.clone(),
                _dcomp_target: target,
                _visual: visual,
                d2d_context,
                _target: target_bitmap,
            })
        }
    }

    /// Dessine la grille et l'état (halo, sélection) puis présente le cadre.
    pub fn render(&self, grid: &Grid, origin: (i32, i32), state: &OverlayState) -> Result<()> {
        unsafe {
            self.d2d_context.BeginDraw();
            self.d2d_context.Clear(None);
            self.draw_grid(grid, origin);
            if let Some(halo) = &state.halo {
                self.draw_halo(grid, origin, *halo)?;
            }
            if let Some(selection) = &state.selection {
                self.draw_selection(origin, *selection)?;
            }
            self.d2d_context.EndDraw(None, None)?;
            let _ = self.swapchain.Present(1, DXGI_PRESENT(0));
            self.dcomp.Commit()?;
        }
        Ok(())
    }

    unsafe fn draw_grid(&self, grid: &Grid, origin: (i32, i32)) {
        let inner = grid.inner();
        let ox = origin.0 as f32;
        let oy = origin.1 as f32;
        let top = (inner.y - origin.1) as f32;
        let bottom = (inner.bottom() - origin.1) as f32;
        let left = (inner.x - origin.0) as f32;
        let right = (inner.right() - origin.0) as f32;

        let line = self
            .solid(1.0, 1.0, 1.0, GRID_LINE_ALPHA)
            .expect("brush ligne");
        let dot = self
            .solid(1.0, 1.0, 1.0, GRID_DOT_ALPHA)
            .expect("brush intersection");
        self.d2d_context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);

        // Traits verticaux et horizontaux, 1 px physique.
        for index in 0..=grid.cols() {
            let x = (grid.column_x(index) as f32 - ox).round();
            self.d2d_context.FillRectangle(
                &D2D_RECT_F {
                    left: x,
                    top,
                    right: x + 1.0,
                    bottom,
                },
                &line,
            );
        }
        for index in 0..=grid.rows() {
            let y = (grid.row_y(index) as f32 - oy).round();
            self.d2d_context.FillRectangle(
                &D2D_RECT_F {
                    left,
                    top: y,
                    right,
                    bottom: y + 1.0,
                },
                &line,
            );
        }

        // Intersections légèrement plus marquées.
        for c in 0..=grid.cols() {
            for r in 0..=grid.rows() {
                let x = (grid.column_x(c) as f32 - ox).round();
                let y = (grid.row_y(r) as f32 - oy).round();
                self.d2d_context.FillRectangle(
                    &D2D_RECT_F {
                        left: x - 1.0,
                        top: y - 1.0,
                        right: x + 1.0,
                        bottom: y + 1.0,
                    },
                    &dot,
                );
            }
        }
    }

    unsafe fn draw_halo(&self, grid: &Grid, origin: (i32, i32), halo: HaloState) -> Result<()> {
        let area = grid.area();
        let left = (area.x - origin.0) as f32;
        let top = (area.y - origin.1) as f32;
        let right = (area.right() - origin.0) as f32;
        let bottom = (area.bottom() - origin.1) as f32;
        let thickness = halo.thickness;

        let (start, end, rect) = match halo.edge {
            ScreenEdge::Left => (
                Vector2 { X: left, Y: top },
                Vector2 {
                    X: left + thickness,
                    Y: top,
                },
                D2D_RECT_F {
                    left,
                    top,
                    right: left + thickness,
                    bottom,
                },
            ),
            ScreenEdge::Right => (
                Vector2 { X: right, Y: top },
                Vector2 {
                    X: right - thickness,
                    Y: top,
                },
                D2D_RECT_F {
                    left: right - thickness,
                    top,
                    right,
                    bottom,
                },
            ),
            ScreenEdge::Top => (
                Vector2 { X: left, Y: top },
                Vector2 {
                    X: left,
                    Y: top + thickness,
                },
                D2D_RECT_F {
                    left,
                    top,
                    right,
                    bottom: top + thickness,
                },
            ),
            ScreenEdge::Bottom => (
                Vector2 { X: left, Y: bottom },
                Vector2 {
                    X: left,
                    Y: bottom - thickness,
                },
                D2D_RECT_F {
                    left,
                    top: bottom - thickness,
                    right,
                    bottom,
                },
            ),
        };

        let brush = self.gradient(start, end)?;
        self.d2d_context.FillRectangle(&rect, &brush);
        Ok(())
    }

    unsafe fn draw_selection(&self, origin: (i32, i32), selection: Rect) -> Result<()> {
        let rounded = D2D1_ROUNDED_RECT {
            rect: rect_to_d2d(selection, origin),
            radiusX: SELECTION_RADIUS,
            radiusY: SELECTION_RADIUS,
        };
        let fill = self.accent(SELECTION_FILL_ALPHA)?;
        let border = self.accent(SELECTION_BORDER_ALPHA)?;
        self.d2d_context.FillRoundedRectangle(&rounded, &fill);
        self.d2d_context
            .DrawRoundedRectangle(&rounded, &border, SELECTION_BORDER_WIDTH, None);
        Ok(())
    }

    unsafe fn solid(&self, r: f32, g: f32, b: f32, a: f32) -> Result<ID2D1SolidColorBrush> {
        self.d2d_context
            .CreateSolidColorBrush(&D2D1_COLOR_F { r, g, b, a }, None)
    }

    unsafe fn accent(&self, a: f32) -> Result<ID2D1SolidColorBrush> {
        self.solid(ACCENT.r, ACCENT.g, ACCENT.b, a)
    }

    unsafe fn gradient(&self, start: Vector2, end: Vector2) -> Result<ID2D1LinearGradientBrush> {
            let stops = [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: D2D1_COLOR_F {
                        r: ACCENT.r,
                        g: ACCENT.g,
                        b: ACCENT.b,
                        a: 1.0,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: D2D1_COLOR_F {
                        r: ACCENT.r,
                        g: ACCENT.g,
                        b: ACCENT.b,
                        a: 0.0,
                    },
                },
            ];
            let collection: ID2D1GradientStopCollection1 = self.d2d_context
                .CreateGradientStopCollection(
                    &stops,
                    D2D1_COLOR_SPACE_SRGB,
                    D2D1_COLOR_SPACE_SRGB,
                    D2D1_BUFFER_PRECISION_8BPC_UNORM,
                    D2D1_EXTEND_MODE_CLAMP,
                    D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT,
                )?;
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: start,
                endPoint: end,
            };
            self.d2d_context
                .CreateLinearGradientBrush(&props, None, &collection)
    }
}

fn rect_to_d2d(rect: Rect, origin: (i32, i32)) -> D2D_RECT_F {
    D2D_RECT_F {
        left: (rect.x - origin.0) as f32,
        top: (rect.y - origin.1) as f32,
        right: (rect.right() - origin.0) as f32,
        bottom: (rect.bottom() - origin.1) as f32,
    }
}
