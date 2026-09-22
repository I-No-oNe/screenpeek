use crate::capture::Region;
use image::RgbaImage;

/// Past this much change, a full read beats stitching bands together.
const FULL_REDRAW_FRACTION: f64 = 0.55;

/// Changed bands grow by this much so glyphs are not cut in half.
const DIRTY_MARGIN: u32 = 16;

/// Bands closer than this are read as one.
const BAND_GAP: u32 = 48;

/// Return changed regions, merging nearby rows and trimming unchanged columns.
pub(super) fn dirty_areas(before: &RgbaImage, after: &RgbaImage) -> Vec<Region> {
    let width = before.width();
    let height = before.height();
    let stride = width as usize * 4;
    let (before, after) = (before.as_raw(), after.as_raw());

    fn row(image: &[u8], index: usize, stride: usize) -> &[u8] {
        &image[index * stride..][..stride]
    }
    let changed = |index: usize| row(before, index, stride) != row(after, index, stride);

    let mut bands: Vec<(u32, u32)> = Vec::new();
    for index in 0..height as usize {
        if !changed(index) {
            continue;
        }
        let index = index as u32;
        match bands.last_mut() {
            Some((_, end)) if index - *end <= BAND_GAP => *end = index,
            _ => bands.push((index, index)),
        }
    }

    bands
        .into_iter()
        .map(|(start, end)| {
            let (mut first, mut last) = (width, 0);
            for index in start..=end {
                if !changed(index as usize) {
                    continue;
                }
                let old = row(before, index as usize, stride);
                let new = row(after, index as usize, stride);
                for column in 0..width {
                    let pixel = column as usize * 4;
                    if old[pixel..pixel + 4] != new[pixel..pixel + 4] {
                        first = first.min(column);
                        last = last.max(column);
                    }
                }
            }

            let left = first.saturating_sub(DIRTY_MARGIN);
            let right = (last + DIRTY_MARGIN + 1).min(width);
            let top = start.saturating_sub(DIRTY_MARGIN);
            let bottom = (end + DIRTY_MARGIN + 1).min(height);

            Region {
                x: left as i32,
                y: top as i32,
                width: right.saturating_sub(left).max(1),
                height: bottom - top,
            }
        })
        .collect()
}

/// Merge changed bands to pay the OCR detection startup cost once.
pub(super) fn merge_bands(bands: &[Region]) -> Option<Region> {
    let left = bands.iter().map(|band| band.x).min()?;
    let top = bands.iter().map(|band| band.y).min()?;
    let right = bands.iter().map(|band| band.x + band.width as i32).max()?;
    let bottom = bands.iter().map(|band| band.y + band.height as i32).max()?;
    Some(Region {
        x: left,
        y: top,
        width: (right - left).max(1) as u32,
        height: (bottom - top).max(1) as u32,
    })
}

/// Choose a patched read when the merged band is small enough.
pub(super) fn worth_patching(bands: &[Region], image: &RgbaImage) -> bool {
    let changed: u32 = bands.iter().map(|band| band.height).sum();
    f64::from(changed) / f64::from(image.height()) < FULL_REDRAW_FRACTION
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u32, height: u32, fill: u8) -> RgbaImage {
        RgbaImage::from_pixel(width, height, image::Rgba([fill, fill, fill, 255]))
    }

    fn paint_row(image: &mut RgbaImage, y: u32) {
        for x in 0..image.width() {
            image.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
        }
    }

    #[test]
    fn merging_covers_every_band() {
        let bands = [
            Region {
                x: 10,
                y: 100,
                width: 50,
                height: 20,
            },
            Region {
                x: 0,
                y: 500,
                width: 200,
                height: 40,
            },
        ];
        let merged = merge_bands(&bands).unwrap();
        assert_eq!(merged.x, 0);
        assert_eq!(merged.y, 100);
        assert_eq!(merged.x + merged.width as i32, 200);
        assert_eq!(merged.y + merged.height as i32, 540);
        assert_eq!(merge_bands(&[]), None);
    }

    #[test]
    fn identical_frames_have_no_changed_areas() {
        assert!(dirty_areas(&frame(64, 64, 0), &frame(64, 64, 0)).is_empty());
    }

    #[test]
    fn a_changed_row_becomes_one_band_with_a_margin() {
        let before = frame(64, 400, 0);
        let mut after = before.clone();
        paint_row(&mut after, 200);

        let bands = dirty_areas(&before, &after);
        assert_eq!(bands.len(), 1);
        assert_eq!(bands[0].y, 200 - DIRTY_MARGIN as i32);
        assert_eq!(bands[0].height, 2 * DIRTY_MARGIN + 1);
        assert_eq!(bands[0].width, 64);
    }

    #[test]
    fn distant_changes_stay_in_separate_bands() {
        let before = frame(64, 600, 0);
        let mut after = before.clone();
        paint_row(&mut after, 20);
        paint_row(&mut after, 500);

        let bands = dirty_areas(&before, &after);
        assert_eq!(bands.len(), 2, "{bands:?}");
        assert!(bands[0].y < bands[1].y);
    }

    #[test]
    fn nearby_changes_merge_into_one_band() {
        let before = frame(64, 600, 0);
        let mut after = before.clone();
        paint_row(&mut after, 100);
        paint_row(&mut after, 100 + BAND_GAP - 1);

        assert_eq!(dirty_areas(&before, &after).len(), 1);
    }

    #[test]
    fn patching_is_abandoned_once_most_of_the_screen_changed() {
        let image = frame(100, 100, 0);
        let band = |height| Region {
            x: 0,
            y: 0,
            width: 100,
            height,
        };
        assert!(worth_patching(&[band(10)], &image));
        assert!(!worth_patching(&[band(30), band(30), band(30)], &image));
    }
}
