//! The photo grid's geometry: sections of justified rows, where every row
//! fills the full width and each photo keeps its real shape. Also answers
//! "which photo is above this one?" for the arrow keys. Pure: sizes in,
//! positions out, in logical pixels.

/// A run of photos shown under one heading: a folder, or an outing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    /// Indices into the library's photos, in display order.
    pub photos: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// One photo's place in a row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub photo: usize,
    pub x: f32,
    pub width: f32,
}

/// What the scrolling list shows, top to bottom.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Header { section: usize },
    Row { height: f32, items: Vec<Placed> },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// Width available to rows.
    pub width: f32,
    /// Rows are filled until they'd be shorter than this.
    pub target_height: f32,
    pub gap: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    blocks: Vec<Block>,
    /// Every shown photo in reading order.
    order: Vec<usize>,
    /// For each library photo, its place in `order` and the row block that
    /// holds it. `None` for photos this view doesn't show.
    place_of_photo: Vec<Option<(usize, usize)>>,
}

impl Layout {
    /// Lays out every section. `aspects[i]` is library photo `i`'s width over
    /// height, for all photos, shown or not.
    pub fn new(sections: &[Section], aspects: &[f32], metrics: Metrics) -> Self {
        let width = metrics.width.max(1.);
        let mut layout = Self { blocks: Vec::new(), order: Vec::new(), place_of_photo: vec![None; aspects.len()] };

        for (section_index, section) in sections.iter().enumerate() {
            layout.blocks.push(Block::Header { section: section_index });
            let mut row: Vec<usize> = Vec::new();
            let mut aspect_sum = 0.;
            for &photo in &section.photos {
                row.push(photo);
                aspect_sum += aspects[photo];
                let gaps = metrics.gap * (row.len() - 1) as f32;
                let height = (width - gaps) / aspect_sum;
                if height <= metrics.target_height {
                    layout.push_row(&row, aspects, height, metrics.gap);
                    row.clear();
                    aspect_sum = 0.;
                }
            }
            // The last row keeps the target height instead of stretching
            // a few photos across the whole width.
            if !row.is_empty() {
                layout.push_row(&row, aspects, metrics.target_height, metrics.gap);
            }
        }
        layout
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// The photos this view shows, in reading order.
    pub fn order(&self) -> &[usize] {
        &self.order
    }

    pub fn contains(&self, photo: usize) -> bool {
        self.place(photo).is_some()
    }

    /// The block that shows `photo`, for scrolling it into view.
    pub fn block_of(&self, photo: usize) -> Option<usize> {
        self.place(photo).map(|(_, block)| block)
    }

    /// The first photo in or after `block`, so thumbnail loading can start
    /// where the user is looking.
    pub fn first_photo_from(&self, block: usize) -> Option<usize> {
        self.blocks.iter().skip(block).find_map(|b| match b {
            Block::Row { items, .. } => items.first().map(|p| p.photo),
            Block::Header { .. } => None,
        })
    }

    /// Where the selection goes from `photo`. Left and right follow reading
    /// order across rows and sections. Up and down jump to the photo in the
    /// neighbouring row whose center is horizontally closest. A photo this
    /// view doesn't show moves to the view's first photo.
    pub fn neighbor(&self, photo: usize, direction: Direction) -> usize {
        let Some((position, here)) = self.place(photo) else {
            return self.order.first().copied().unwrap_or(photo);
        };
        match direction {
            Direction::Left => self.order[position.saturating_sub(1)],
            Direction::Right => self.order[(position + 1).min(self.order.len() - 1)],
            Direction::Up | Direction::Down => {
                let Some(center) = self.center_of(photo, here) else { return photo };
                let rows: Box<dyn Iterator<Item = &Block>> = match direction {
                    Direction::Up => Box::new(self.blocks[..here].iter().rev()),
                    _ => Box::new(self.blocks[here + 1..].iter()),
                };
                rows.filter_map(|b| match b {
                    Block::Row { items, .. } => Some(items),
                    Block::Header { .. } => None,
                })
                .next()
                .and_then(|items| {
                    items.iter().min_by(|a, b| {
                        let da = (a.x + a.width / 2. - center).abs();
                        let db = (b.x + b.width / 2. - center).abs();
                        da.total_cmp(&db)
                    })
                })
                .map_or(photo, |p| p.photo)
            }
        }
    }

    /// Where `photo` sits in `order`, the view's reading order.
    pub fn position(&self, photo: usize) -> Option<usize> {
        self.place(photo).map(|(position, _)| position)
    }

    fn place(&self, photo: usize) -> Option<(usize, usize)> {
        self.place_of_photo.get(photo).copied().flatten()
    }

    fn center_of(&self, photo: usize, block: usize) -> Option<f32> {
        match self.blocks.get(block)? {
            Block::Row { items, .. } => items.iter().find(|p| p.photo == photo).map(|p| p.x + p.width / 2.),
            Block::Header { .. } => None,
        }
    }

    fn push_row(&mut self, row: &[usize], aspects: &[f32], height: f32, gap: f32) {
        let block = self.blocks.len();
        let mut x = 0.;
        let mut items = Vec::with_capacity(row.len());
        for &photo in row {
            let width = aspects[photo] * height;
            items.push(Placed { photo, x, width });
            x += width + gap;
            self.place_of_photo[photo] = Some((self.order.len(), block));
            self.order.push(photo);
        }
        self.blocks.push(Block::Row { height, items });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const METRICS: Metrics = Metrics { width: 1000., target_height: 200., gap: 0. };

    fn rows(layout: &Layout) -> Vec<Vec<usize>> {
        layout
            .blocks()
            .iter()
            .filter_map(|b| match b {
                Block::Row { items, .. } => Some(items.iter().map(|p| p.photo).collect()),
                Block::Header { .. } => None,
            })
            .collect()
    }

    fn section(photos: std::ops::Range<usize>) -> Section {
        Section { title: "a".into(), photos: photos.collect() }
    }

    #[test]
    fn rows_fill_the_width_and_keep_each_photo_shape() {
        // Five 3:2 photos at width 1000: three fit (h = 222 > 200), four
        // give h = 166.7, so the first row has four.
        let sections = [section(0..5)];
        let layout = Layout::new(&sections, &[1.5; 5], METRICS);
        assert_eq!(rows(&layout), vec![vec![0, 1, 2, 3], vec![4]]);

        let Block::Row { height, items } = &layout.blocks()[1] else { panic!("expected a row") };
        let right_edge = items.last().map(|p| p.x + p.width).unwrap();
        assert!((right_edge - 1000.).abs() < 0.01, "row ends at {right_edge}");
        assert!((items[0].width / height - 1.5).abs() < 0.001);

        let Block::Row { height, .. } = &layout.blocks()[2] else { panic!("expected a row") };
        assert_eq!(*height, 200., "the last row isn't stretched");
    }

    #[test]
    fn each_section_starts_new_rows_under_its_header() {
        let sections = [section(0..2), section(2..3)];
        let layout = Layout::new(&sections, &[1.5; 3], METRICS);
        assert!(matches!(layout.blocks()[0], Block::Header { section: 0 }));
        assert!(matches!(layout.blocks()[2], Block::Header { section: 1 }));
        assert_eq!(rows(&layout), vec![vec![0, 1], vec![2]]);
        assert_eq!(layout.block_of(2), Some(3));
        assert_eq!(layout.first_photo_from(2), Some(2));
    }

    #[test]
    fn up_and_down_pick_the_horizontally_closest_photo() {
        // Row 1: a wide panorama then a portrait. Row 2: four 3:2 photos.
        //  [ 0 (wide)        ][1]
        //  [ 2 ][ 3 ][ 4 ][ 5 ]
        let sections = [section(0..6)];
        let aspects = [4.5, 0.5, 1.5, 1.5, 1.5, 1.5];
        let layout = Layout::new(&sections, &aspects, METRICS);
        assert_eq!(rows(&layout), vec![vec![0, 1], vec![2, 3, 4, 5]]);

        assert_eq!(layout.neighbor(1, Direction::Down), 5);
        assert_eq!(layout.neighbor(0, Direction::Down), 3);
        assert_eq!(layout.neighbor(3, Direction::Up), 0);
        assert_eq!(layout.neighbor(5, Direction::Up), 1);
        assert_eq!(layout.neighbor(0, Direction::Up), 0, "stays on the top row");
        assert_eq!(layout.neighbor(4, Direction::Down), 4, "stays on the bottom row");
    }

    #[test]
    fn up_and_down_cross_section_headers_and_left_right_follow_reading_order() {
        let sections = [section(0..1), section(1..3)];
        let layout = Layout::new(&sections, &[1.5; 3], METRICS);
        assert_eq!(layout.neighbor(0, Direction::Down), 1);
        assert_eq!(layout.neighbor(2, Direction::Up), 0);
        assert_eq!(layout.neighbor(0, Direction::Left), 0);
        assert_eq!(layout.neighbor(0, Direction::Right), 1);
        assert_eq!(layout.neighbor(2, Direction::Right), 2);
    }

    #[test]
    fn a_view_can_show_some_photos_in_any_order() {
        // Photos 3, 0 and 2 in that order; photo 1 isn't shown.
        let sections = [Section { title: "a".into(), photos: vec![3, 0, 2] }];
        let layout = Layout::new(&sections, &[1.5; 4], METRICS);
        assert_eq!(layout.order(), &[3, 0, 2]);
        assert_eq!(layout.neighbor(3, Direction::Right), 0);
        assert_eq!(layout.neighbor(2, Direction::Left), 0);
        assert!(!layout.contains(1));
        assert_eq!(layout.neighbor(1, Direction::Right), 3, "a hidden photo jumps to the first shown");
    }
}
