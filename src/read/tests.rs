use super::*;
use std::time::Instant;

#[test]
#[ignore = "loads OCR models; run cargo test --release ocr_fixture -- --ignored --nocapture"]
fn ocr_fixture_cache_accuracy_and_speed() {
    let engine = Engine::load().unwrap();
    let image = image::open("bench/fixtures/dialog.png")
        .unwrap()
        .into_rgba8();
    let capture = Capture::from_image(image);
    let started = Instant::now();
    let cold = engine.read(&capture).unwrap();
    let cold_ms = started.elapsed().as_secs_f64() * 1000.0;
    let expected: Vec<_> = include_str!("../../bench/fixtures/dialog.expected")
        .lines()
        .collect();
    let hits = expected
        .iter()
        .filter(|text| cold.iter().any(|e| &e.text == *text))
        .count();
    assert!(
        hits * 100 / expected.len() >= 90,
        "only {hits}/{} labels recognized: {cold:?}",
        expected.len()
    );
    let mut timings = Vec::new();
    for _ in 0..7 {
        let started = Instant::now();
        let warm = engine.read(&capture).unwrap();
        timings.push(started.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(
            serde_json::to_value(&warm).unwrap(),
            serde_json::to_value(&cold).unwrap()
        );
    }
    timings.sort_by(f64::total_cmp);
    println!(
        "OCR fixture: {hits}/{} labels; cold {cold_ms:.1} ms; warm median {:.1} ms",
        expected.len(),
        timings[3]
    );
    if let Ok(limit) = std::env::var("SCREENPEEK_MAX_WARM_MS") {
        assert!(
            timings[3] <= limit.parse::<f64>().unwrap(),
            "warm OCR exceeded {limit} ms"
        );
    }

    {
        let mut cache = engine.lines.lock().unwrap();
        let missing = *cache.keys().next().unwrap();
        cache.remove(&missing);
        cache.extend((0..LINE_CACHE_SIZE as u64).map(|key| (key, "unused".into())));
    }
    assert_eq!(
        serde_json::to_value(engine.read(&capture).unwrap()).unwrap(),
        serde_json::to_value(&cold).unwrap()
    );

    // Repeated pixels at different positions used to shift subsequent text.
    let mut doubled = image::RgbaImage::new(capture.image.width(), capture.image.height() * 2);
    image::imageops::replace(&mut doubled, &capture.image, 0, 0);
    image::imageops::replace(
        &mut doubled,
        &capture.image,
        0,
        capture.image.height() as i64,
    );
    let doubled = Capture::from_image(doubled);
    engine.lines.lock().unwrap().clear();
    let first = engine.read(&doubled).unwrap();
    let second = engine.read(&doubled).unwrap();
    assert!(!first.is_empty());
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(&second).unwrap()
    );
    let midpoint = capture.image.height() as i32;
    let top: Vec<_> = first
        .iter()
        .filter(|e| e.y < midpoint)
        .map(|e| &e.text)
        .collect();
    let bottom: Vec<_> = first
        .iter()
        .filter(|e| e.y >= midpoint)
        .map(|e| &e.text)
        .collect();
    for half in [top, bottom] {
        let hits = expected
            .iter()
            .filter(|label| half.iter().any(|text| *text == *label))
            .count();
        assert!(
            hits * 100 / expected.len() >= 90,
            "duplicated fixture lost labels: {half:?}"
        );
    }
    engine.lines.lock().unwrap().clear();
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                let result = engine.read(&doubled).unwrap();
                assert_eq!(
                    serde_json::to_value(&result).unwrap(),
                    serde_json::to_value(&second).unwrap()
                );
            });
        }
    });
}

#[test]
fn line_hash_tracks_pixels_and_size_but_not_position() {
    let mut image = image::RgbaImage::from_pixel(200, 100, image::Rgba([50, 60, 70, 255]));
    let line = |x, y, w, h| [RotatedRect::from_rect(Rect::from_tlhw(y, x, h, w))];
    line_key(&image, &line(300., 300., 80., 20.));
    let original = line_key(&image, &line(5., 5., 80., 20.)).0;
    assert_eq!(original, line_key(&image, &line(100., 50., 80., 20.)).0);
    assert_ne!(original, line_key(&image, &line(5., 5., 81., 20.)).0);
    image.put_pixel(10, 10, image::Rgba([51, 60, 70, 255]));
    assert_ne!(original, line_key(&image, &line(5., 5., 80., 20.)).0);
}

#[test]
fn wide_gaps_split_controls_but_keep_words_together() {
    let word = |x, width| RotatedRect::from_rect(Rect::from_tlhw(0., x, 12., width));
    let lines = separate_controls(&[vec![word(0., 30.), word(35., 20.), word(120., 40.)]], &[]);
    assert_eq!(lines.iter().map(Vec::len).collect::<Vec<_>>(), vec![2, 1]);
    assert_eq!(lines[1][0].bounding_rect().left(), 120.);
}

/// Adjacent toolbar buttons: 13 px caps 13 px apart used to merge under the old
/// `2.0 * height` gap, which is what `bench/fixtures/dense.png` exposes.
#[test]
fn button_padding_splits_adjacent_labels() {
    let word = |x, width| RotatedRect::from_rect(Rect::from_tlhw(0., x, 13., width));
    let lines = separate_controls(&[vec![word(24., 29.), word(66., 31.)]], &[]);
    assert_eq!(lines.len(), 2);
}

#[test]
fn different_shapes_split_even_when_adjacent() {
    let word =
        |top, x, height, width| RotatedRect::from_rect(Rect::from_tlhw(top, x, height, width));
    // Touching, but twice the height: a heading beside body text.
    let by_height = separate_controls(
        &[vec![word(0., 0., 24., 30.), word(0., 32., 12., 20.)]],
        &[],
    );
    assert_eq!(by_height.len(), 2);
    // Same height, baselines far enough apart to be different controls.
    let by_centre = separate_controls(
        &[vec![word(0., 0., 12., 30.), word(9., 32., 12., 20.)]],
        &[],
    );
    assert_eq!(by_centre.len(), 2);
    // Same height and baseline, one space apart: still one label.
    let together = separate_controls(
        &[vec![word(0., 0., 12., 30.), word(0., 34., 12., 20.)]],
        &[],
    );
    assert_eq!(together.len(), 1);
}

/// Split adjacent Save/Cancel controls at the window boundary.
#[test]
fn a_window_edge_splits_controls_a_word_space_apart() {
    let word = |x, width| RotatedRect::from_rect(Rect::from_tlhw(0., x, 13., width));
    let line = vec![word(396., 29.), word(433., 41.)];

    assert_eq!(separate_controls(std::slice::from_ref(&line), &[]).len(), 1);
    assert_eq!(
        separate_controls(std::slice::from_ref(&line), &[430.]).len(),
        2
    );
    // An edge somewhere else leaves the pair alone.
    assert_eq!(separate_controls(&[line], &[200.]).len(), 1);
}
