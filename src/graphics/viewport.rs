#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StageViewport {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl StageViewport {
    // whole 4-by-3 pixel units keep the viewport exactly 4:3.
    pub(crate) fn for_window(width: u32, height: u32) -> Option<Self> {
        let units = (width / 4).min(height / 3);
        if units == 0 {
            return None;
        }
        let viewport_width = units * 4;
        let viewport_height = units * 3;
        Some(Self {
            x: (width - viewport_width) / 2,
            y: (height - viewport_height) / 2,
            width: viewport_width,
            height: viewport_height,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::StageViewport;

    #[test]
    fn viewport_fits_and_centers_at_exact_four_by_three() {
        let cases = [
            (1920, 1080, Some((240, 0, 1440, 1080))),
            (2560, 1440, Some((320, 0, 1920, 1440))),
            (1280, 1024, Some((0, 32, 1280, 960))),
            (1001, 701, Some((34, 1, 932, 699))),
            (0, 360, None),
            (3, 2, None),
        ];

        for (width, height, expected) in cases {
            let actual = StageViewport::for_window(width, height)
                .map(|viewport| (viewport.x, viewport.y, viewport.width, viewport.height));
            assert_eq!(actual, expected, "window size: {width}x{height}");
        }
    }
}
