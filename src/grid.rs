//! Modèle de grille de placement, pur et testable, sans aucune API Win32.
//!
//! Tout est en pixels physiques et en coordonnées d'écran virtuel (un écran à
//! gauche du principal a des coordonnées x négatives). Deux modes :
//! - Fixed : pas fixe en pixels, le reste est centré ou absorbé par la dernière colonne.
//! - Relative : nombre de cellules dérivé d'un pourcentage (5 % donne 20 x 20).

/// Rectangle en pixels physiques, coordonnées d'écran virtuel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Construit depuis les bords, largeur/hauteur bornées à 0.
    pub fn from_edges(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            x: left,
            y: top,
            width: (right - left).max(0) as u32,
            height: (bottom - top).max(0) as u32,
        }
    }

    pub fn right(&self) -> i32 {
        self.x + self.width as i32
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height as i32
    }

    /// Réduit de `dx` à gauche et à droite, de `dy` en haut et en bas.
    /// Retourne None si la marge dépasse la moitié d'une dimension.
    pub fn inset(&self, dx: i32, dy: i32) -> Option<Self> {
        if self.width as i32 <= 2 * dx || self.height as i32 <= 2 * dy {
            return None;
        }
        Some(Self::from_edges(
            self.x + dx,
            self.y + dy,
            self.right() - dx,
            self.bottom() - dy,
        ))
    }

    pub fn inset_left(&self, px: u32) -> Self {
        Self::from_edges(self.x + px as i32, self.y, self.right(), self.bottom())
    }

    pub fn inset_top(&self, px: u32) -> Self {
        Self::from_edges(self.x, self.y + px as i32, self.right(), self.bottom())
    }

    pub fn inset_right(&self, px: u32) -> Self {
        Self::from_edges(self.x, self.y, self.right() - px as i32, self.bottom())
    }

    pub fn inset_bottom(&self, px: u32) -> Self {
        Self::from_edges(self.x, self.y, self.right(), self.bottom() - px as i32)
    }
}

/// Mode de grille d'un écran.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridMode {
    /// Pas fixe en pixels physiques.
    Fixed { pitch_px: u32 },
    /// Pourcentage de la zone pour une colonne ou une ligne.
    Relative { percent: f64 },
}

/// Répartition du reste de division dans le mode Fixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remainder {
    /// Reste réparti équitablement de chaque côté (grille centrée).
    Centered,
    /// Reste absorbé par la dernière colonne et la dernière ligne.
    LastColumn,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridOptions {
    pub mode: GridMode,
    /// Marge extérieure en pixels, autour de la zone de travail.
    pub margin: u32,
    /// Gouttière en pixels entre deux fenêtres placées côte à côte.
    pub gap: u32,
    pub remainder: Remainder,
}

impl Default for GridOptions {
    fn default() -> Self {
        Self {
            mode: GridMode::Relative { percent: 5.0 },
            margin: 0,
            gap: 0,
            remainder: Remainder::Centered,
        }
    }
}

/// Position d'une cellule dans la grille.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub col: u32,
    pub row: u32,
}

/// Plage de cellules, bornes inclusives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRange {
    pub col0: u32,
    pub row0: u32,
    pub col1: u32,
    pub row1: u32,
}

/// Grille calculée sur une zone de travail. Les bornes de colonnes et de lignes
/// sont précalculées en f64 puis arrondies, pour éviter les dérives cumulées.
#[derive(Debug, Clone)]
pub struct Grid {
    area: Rect,
    options: GridOptions,
    cols: u32,
    rows: u32,
    inner: Rect,
    origin_x: f64,
    origin_y: f64,
    slot_w: f64,
    slot_h: f64,
}

impl Grid {
    /// Construit la grille. Retourne None si la zone est trop petite pour la marge.
    pub fn new(area: Rect, options: GridOptions) -> Option<Grid> {
        let margin = options.margin as i32;
        let inner = area.inset(margin, margin)?;
        if inner.width == 0 || inner.height == 0 {
            return None;
        }
        let (cols, slot_w, origin_x) =
            layout(inner.width as f64, inner.x, options.mode, options.remainder);
        let (rows, slot_h, origin_y) =
            layout(inner.height as f64, inner.y, options.mode, options.remainder);
        Some(Grid {
            area,
            options,
            cols,
            rows,
            inner,
            origin_x,
            origin_y,
            slot_w,
            slot_h,
        })
    }

    pub fn area(&self) -> Rect {
        self.area
    }

    /// Zone intérieure de la grille, après la marge.
    pub fn inner(&self) -> Rect {
        self.inner
    }

    pub fn cols(&self) -> u32 {
        self.cols
    }

    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// Position x du trait vertical n°index (0..=cols), coordonnées absolues.
    pub fn column_x(&self, index: u32) -> i32 {
        if index >= self.cols {
            self.col_bounds(self.cols - 1).1
        } else {
            self.col_bounds(index).0
        }
    }

    /// Position y du trait horizontal n°index (0..=rows), coordonnées absolues.
    pub fn row_y(&self, index: u32) -> i32 {
        if index >= self.rows {
            self.row_bounds(self.rows - 1).1
        } else {
            self.row_bounds(index).0
        }
    }

    pub fn cell_rect(&self, cell: Cell) -> Rect {
        let (left, right) = self.col_bounds(cell.col);
        let (top, bottom) = self.row_bounds(cell.row);
        Rect::from_edges(left, top, right, bottom)
    }

    /// Cellule sous le point, None si le point est hors grille.
    pub fn hit_test(&self, x: i32, y: i32) -> Option<Cell> {
        let col = (0..self.cols).find(|&c| {
            let (left, right) = self.col_bounds(c);
            x >= left && x < right
        })?;
        let row = (0..self.rows).find(|&r| {
            let (top, bottom) = self.row_bounds(r);
            y >= top && y < bottom
        })?;
        Some(Cell { col, row })
    }

    /// Plage normalisée entre deux cellules (ordre indifférent).
    pub fn selection(&self, anchor: Cell, current: Cell) -> CellRange {
        let range = CellRange {
            col0: anchor.col.min(current.col),
            col1: anchor.col.max(current.col),
            row0: anchor.row.min(current.row),
            row1: anchor.row.max(current.row),
        };
        self.clamp_range(range)
    }

    /// Ramène la plage dans les bornes de la grille.
    pub fn clamp_range(&self, range: CellRange) -> CellRange {
        CellRange {
            col0: range.col0.min(self.cols - 1),
            col1: range.col1.min(self.cols - 1),
            row0: range.row0.min(self.rows - 1),
            row1: range.row1.min(self.rows - 1),
        }
    }

    /// Rectangle couvrant la plage de cellules, réduit de la gouttière (gap).
    pub fn cell_range_rect(&self, range: CellRange) -> Rect {
        let range = self.clamp_range(range);
        let (left, _) = self.col_bounds(range.col0);
        let (_, right) = self.col_bounds(range.col1);
        let (top, _) = self.row_bounds(range.row0);
        let (_, bottom) = self.row_bounds(range.row1);
        let gap = self.options.gap as i32;
        let left = left + gap / 2;
        let top = top + gap / 2;
        let width = (right - gap / 2 - left).max(1);
        let height = (bottom - gap / 2 - top).max(1);
        Rect::from_edges(left, top, left + width, top + height)
    }

    fn col_bounds(&self, col: u32) -> (i32, i32) {
        let col = col.min(self.cols - 1);
        let (x, w) = match self.options.mode {
            GridMode::Fixed { pitch_px }
                if self.options.remainder == Remainder::LastColumn && col == self.cols - 1 =>
            {
                let pitch = pitch_px.max(1) as f64;
                let start = self.inner.x as f64 + (self.cols - 1) as f64 * pitch;
                let width = self.inner.width as f64 - (self.cols - 1) as f64 * pitch;
                (start, width)
            }
            _ => (self.origin_x + col as f64 * self.slot_w, self.slot_w),
        };
        (x.round() as i32, (x + w).round() as i32)
    }

    fn row_bounds(&self, row: u32) -> (i32, i32) {
        let row = row.min(self.rows - 1);
        let (y, h) = match self.options.mode {
            GridMode::Fixed { pitch_px }
                if self.options.remainder == Remainder::LastColumn && row == self.rows - 1 =>
            {
                let pitch = pitch_px.max(1) as f64;
                let start = self.inner.y as f64 + (self.rows - 1) as f64 * pitch;
                let height = self.inner.height as f64 - (self.rows - 1) as f64 * pitch;
                (start, height)
            }
            _ => (self.origin_y + row as f64 * self.slot_h, self.slot_h),
        };
        (y.round() as i32, (y + h).round() as i32)
    }
}

/// Calcule le nombre de colonnes ou de lignes et la position du premier slot.
fn layout(
    size: f64,
    origin: i32,
    mode: GridMode,
    remainder: Remainder,
) -> (u32, f64, f64) {
    match mode {
        GridMode::Fixed { pitch_px } => {
            let pitch = pitch_px.max(1) as f64;
            let count_raw = (size / pitch) as u32;
            if count_raw == 0 {
                // Le pas dépasse la taille, une seule cellule couvre toute la zone.
                return (1, size, origin as f64);
            }
            let total = count_raw as f64 * pitch;
            let leftover = size - total;
            let offset = if remainder == Remainder::Centered {
                leftover / 2.0
            } else {
                0.0
            };
            (count_raw, pitch, origin as f64 + offset)
        }
        GridMode::Relative { percent } => {
            let pct = percent.clamp(1.0, 50.0);
            let count = ((100.0 / pct).round() as u32).max(1);
            let slot = size / count as f64;
            (count, slot, origin as f64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(area: Rect, mode: GridMode, remainder: Remainder) -> Grid {
        Grid::new(
            area,
            GridOptions {
                mode,
                margin: 0,
                gap: 0,
                remainder,
                ..Default::default()
            },
        )
        .expect("grille valide")
    }

    #[test]
    fn fixed_centered_sans_reste() {
        let g = grid(
            Rect::new(0, 0, 2000, 1000),
            GridMode::Fixed { pitch_px: 100 },
            Remainder::Centered,
        );
        assert_eq!((g.cols(), g.rows()), (20, 10));
        assert_eq!(g.cell_rect(Cell { col: 0, row: 0 }), Rect::new(0, 0, 100, 100));
        assert_eq!(
            g.cell_rect(Cell { col: 19, row: 9 }),
            Rect::new(1900, 900, 100, 100)
        );
    }

    #[test]
    fn fixed_centered_avec_reste() {
        let g = grid(
            Rect::new(0, 0, 1920, 1080),
            GridMode::Fixed { pitch_px: 100 },
            Remainder::Centered,
        );
        assert_eq!((g.cols(), g.rows()), (19, 10));
        // 20 px de reste en largeur (offset 10), 80 en hauteur (offset 40).
        assert_eq!(g.cell_rect(Cell { col: 0, row: 0 }), Rect::new(10, 40, 100, 100));
        assert_eq!(
            g.cell_rect(Cell { col: 18, row: 9 }),
            Rect::new(1810, 940, 100, 100)
        );
    }

    #[test]
    fn fixed_derniere_colonne_absorbe_le_reste() {
        let g = grid(
            Rect::new(0, 0, 1920, 1080),
            GridMode::Fixed { pitch_px: 100 },
            Remainder::LastColumn,
        );
        assert_eq!(
            g.cell_rect(Cell { col: 18, row: 9 }),
            Rect::new(1800, 900, 120, 180)
        );
    }

    #[test]
    fn ecran_a_gauche_du_principal() {
        let g = grid(
            Rect::new(-1920, 0, 1920, 1080),
            GridMode::Fixed { pitch_px: 100 },
            Remainder::Centered,
        );
        assert_eq!(
            g.cell_rect(Cell { col: 0, row: 0 }),
            Rect::new(-1910, 40, 100, 100)
        );
        assert_eq!(
            g.cell_rect(Cell { col: 18, row: 9 }),
            Rect::new(-110, 940, 100, 100)
        );
    }

    #[test]
    fn relative_par_defaut_20x20() {
        let g = grid(
            Rect::new(0, 0, 3840, 2160),
            GridMode::Relative { percent: 5.0 },
            Remainder::Centered,
        );
        assert_eq!((g.cols(), g.rows()), (20, 20));
        assert_eq!(g.cell_rect(Cell { col: 0, row: 0 }), Rect::new(0, 0, 192, 108));
        assert_eq!(
            g.cell_rect(Cell { col: 19, row: 19 }),
            Rect::new(3648, 2052, 192, 108)
        );
    }

    #[test]
    fn hit_test_dans_et_hors_grille() {
        let g = grid(
            Rect::new(0, 0, 2000, 1000),
            GridMode::Fixed { pitch_px: 100 },
            Remainder::Centered,
        );
        assert_eq!(g.hit_test(550, 250), Some(Cell { col: 5, row: 2 }));
        assert_eq!(g.hit_test(-1, 0), None);
        assert_eq!(g.hit_test(2000, 0), None);
    }

    #[test]
    fn gouttiere_reduit_le_rectangle() {
        let g = Grid::new(
            Rect::new(0, 0, 2000, 1000),
            GridOptions {
                mode: GridMode::Fixed { pitch_px: 100 },
                margin: 0,
                gap: 8,
                remainder: Remainder::Centered,
            },
        )
        .unwrap();
        let rect = g.cell_range_rect(CellRange {
            col0: 0,
            row0: 0,
            col1: 1,
            row1: 1,
        });
        assert_eq!(rect, Rect::new(4, 4, 192, 192));
    }

    #[test]
    fn marge_decale_la_grille() {
        let g = Grid::new(
            Rect::new(0, 0, 2000, 1000),
            GridOptions {
                mode: GridMode::Fixed { pitch_px: 100 },
                margin: 50,
                gap: 0,
                remainder: Remainder::Centered,
            },
        )
        .unwrap();
        assert_eq!(g.cell_rect(Cell { col: 0, row: 0 }), Rect::new(50, 50, 100, 100));
        assert_eq!(g.cols(), 19);
    }

    #[test]
    fn pas_plus_grand_que_la_zone() {
        let g = grid(
            Rect::new(0, 0, 50, 50),
            GridMode::Fixed { pitch_px: 100 },
            Remainder::Centered,
        );
        assert_eq!((g.cols(), g.rows()), (1, 1));
        assert_eq!(g.cell_rect(Cell { col: 0, row: 0 }), Rect::new(0, 0, 50, 50));
    }

    #[test]
    fn selection_normalise_les_bornes() {
        let g = grid(
            Rect::new(0, 0, 2000, 1000),
            GridMode::Fixed { pitch_px: 100 },
            Remainder::Centered,
        );
        let range = g.selection(Cell { col: 5, row: 5 }, Cell { col: 2, row: 3 });
        assert_eq!(
            range,
            CellRange {
                col0: 2,
                row0: 3,
                col1: 5,
                row1: 5
            }
        );
    }

    #[test]
    fn plage_bornee_par_la_grille() {
        let g = grid(
            Rect::new(0, 0, 2000, 1000),
            GridMode::Fixed { pitch_px: 100 },
            Remainder::Centered,
        );
        let rect = g.cell_range_rect(CellRange {
            col0: 0,
            row0: 0,
            col1: 99,
            row1: 99,
        });
        assert_eq!(rect, Rect::new(0, 0, 2000, 1000));
    }

    #[test]
    fn marge_trop_grande_est_invalide() {
        let result = Grid::new(
            Rect::new(0, 0, 100, 100),
            GridOptions {
                mode: GridMode::Fixed { pitch_px: 10 },
                margin: 60,
                gap: 0,
                remainder: Remainder::Centered,
            },
        );
        assert!(result.is_none());
    }
}
