use super::*;

/// A tree that names what recognition finds may stand in for those pixels;
/// one that names a fraction of it may not, however confidently it reports.
#[test]
fn only_a_tree_that_accounts_for_the_pixels_is_verified() {
    let rect = Region {
        x: 0,
        y: 0,
        width: 200,
        height: 100,
    };
    let at = |text: &str, x: i32| Element {
        id: 0,
        text: text.to_owned(),
        x,
        y: 10,
        width: 40,
        height: 12,
        source: crate::index::Source::Ocr,
    };
    let recognized = vec![at("Save", 10), at("Cancel", 60), at("Apply", 110)];

    let window = |claims: &[&str]| Located {
        rect,
        elements: claims.iter().map(|text| at(text, 0)).collect(),
        key: Some(("Dialog".into(), 200, 100)),
        verified: false,
    };

    let verdict = |claims: &[&str], recognized: &[Element]| {
        let mut covered = HashMap::new();
        verify_coverage(&mut covered, &[window(claims)], recognized);
        covered[&("Dialog".to_string(), 200, 100)]
    };

    assert!(verdict(&["Save", "Cancel", "Apply"], &recognized));
    assert!(!verdict(&["Dialog"], &recognized));
    // A window with nothing recognized in it proves nothing either way.
    assert!(!verdict(&["Save"], &[]));
}

/// Changed pixels inside an unverified window are still read, or a tree
/// that reports only a title would hide everything drawn beneath it.
#[test]
fn unverified_windows_do_not_swallow_changed_pixels() {
    let area = Region {
        x: 10,
        y: 10,
        width: 50,
        height: 20,
    };
    let window = |verified| Located {
        rect: Region {
            x: 0,
            y: 0,
            width: 200,
            height: 100,
        },
        elements: Vec::new(),
        key: None,
        verified,
    };
    assert_eq!(outside(&[area], &[window(false)], (0, 0)), vec![area]);
    assert!(outside(&[area], &[window(true)], (0, 0)).is_empty());
}

#[test]
#[ignore = "loads OCR models; run cargo test --release --bin screenpeek -- --ignored --nocapture"]
fn patch_preserves_duplicate_bands_across_cache_eviction() {
    let image = image::open("bench/fixtures/dialog.png")
        .unwrap()
        .into_rgba8();
    let (width, height) = image.dimensions();
    let engine = Engine::load().unwrap();
    let expected = engine.read(&Capture::from_image(image.clone())).unwrap();
    let mut doubled = RgbaImage::new(width, height * 2);
    image::imageops::replace(&mut doubled, &image, 0, 0);
    image::imageops::replace(&mut doubled, &image, 0, height as i64);
    let capture = Capture {
        image: doubled,
        origin: (-100, 50),
    };
    let mut session = Session {
        engine,
        how: "fixture",
        previous: None,
        bands: (0..BAND_CACHE_SIZE as u64 - 1)
            .map(|key| (key, Vec::new()))
            .collect(),
        #[cfg(target_os = "linux")]
        capturer: capture::Backend::Portal,
        #[cfg(target_os = "linux")]
        tree: HashMap::new(),
        covered: HashMap::new(),
    };
    let bands = [
        Region {
            x: 0,
            y: 0,
            width,
            height,
        },
        Region {
            x: 0,
            y: height as i32,
            width,
            height,
        },
    ];
    for _ in 0..2 {
        let result = session
            .patch(Vec::new(), &capture, &bands, &[], None)
            .unwrap();
        assert_eq!(result.len(), expected.len() * 2);
        for offset in [0, height as i32] {
            for element in &expected {
                assert!(result.iter().any(|found| found.text == element.text
                    && found.x == element.x - 100
                    && found.y == element.y + offset + 50));
            }
        }
    }
}
#[test]
#[ignore = "fixture pipeline benchmark; cargo test --release --bin screenpeek -- --ignored --nocapture --test-threads=1"]
fn fixture_scan_scenarios() {
    let image = image::open("bench/fixtures/dialog.png")
        .unwrap()
        .into_rgba8();
    let mut session = Session {
        engine: Engine::load().unwrap(),
        how: "fixture",
        previous: None,
        bands: HashMap::new(),
        #[cfg(target_os = "linux")]
        capturer: capture::Backend::Portal,
        #[cfg(target_os = "linux")]
        tree: HashMap::new(),
        covered: HashMap::new(),
    };
    let request = Request {
        token: String::new(),
        region: None,
        monitor: None,
        session: 0,
        lang: None,
        excluded: Vec::new(),
    };
    // Place fixture far outside the live desktop so live accessibility cannot cover it.
    let origin = (-10000, -10000);
    let capture = || Capture {
        image: image.clone(),
        origin,
    };
    let mut timings = std::collections::BTreeMap::new();
    let mut results = None;
    for scenario in ["full_ocr", "unchanged", "button_region"] {
        let mut samples = Vec::new();
        for _ in 0..7 {
            let input = if scenario == "button_region" {
                capture::crop(
                    capture(),
                    Region {
                        x: origin.0 + 560,
                        y: origin.1 + 470,
                        width: 340,
                        height: 60,
                    },
                )
                .unwrap()
            } else {
                capture()
            };
            if scenario != "unchanged" {
                session.previous = None;
                session.engine.clear_cache_for_test();
            }
            let started = Instant::now();
            let elements = session
                .read_capture(&request, input, Duration::ZERO)
                .unwrap();
            let snapshot = index::Snapshot::new(elements);
            let target = snapshot.find("Save").unwrap();
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
            assert!((origin.0 + 800..origin.0 + 876).contains(&target.x));
            assert!((origin.1 + 480..origin.1 + 514).contains(&target.y));
            if scenario == "button_region" {
                assert_eq!(snapshot.elements.len(), 3);
            }
            results = Some(snapshot);
        }
        samples.sort_by(f64::total_cmp);
        timings.insert(scenario, samples[3]);
    }
    // Removing a label must invalidate it, including after returning to a prior region.
    session
        .read_capture(&request, capture(), Duration::ZERO)
        .unwrap();
    let mut removed = capture();
    for y in 480..515 {
        for x in 800..877 {
            removed
                .image
                .put_pixel(x, y, image::Rgba([246, 246, 246, 255]));
        }
    }
    let changed = session
        .read_capture(&request, removed, Duration::ZERO)
        .unwrap();
    let changed = index::Snapshot::new(changed);
    assert!(changed.find("Save").is_err());
    assert!(changed.find("Cancel").is_ok());
    let restored = session
        .read_capture(&request, capture(), Duration::ZERO)
        .unwrap();
    assert!(index::Snapshot::new(restored).find("Save").is_ok());
    let mut terminal_request = request;
    terminal_request.excluded = vec![Region {
        x: origin.0,
        y: origin.1,
        width: 560,
        height: 560,
    }];
    let masked = session
        .read_capture(&terminal_request, capture(), Duration::ZERO)
        .unwrap();
    assert!(masked.iter().all(|element| element.x >= origin.0 + 560));
    let mut noisy = capture();
    noisy.image.put_pixel(30, 30, image::Rgba([255, 0, 0, 255]));
    let after_noise = session
        .read_capture(&terminal_request, noisy, Duration::ZERO)
        .unwrap();
    assert_eq!(
        serde_json::to_value(&masked).unwrap(),
        serde_json::to_value(&after_noise).unwrap()
    );
    assert_eq!(
        session.previous.as_ref().unwrap().image.get_pixel(30, 30).0,
        [0, 0, 0, 255]
    );
    let result = serde_json::json!({"median_ms": timings, "runs": 7,
        "scope": "model-loaded fixture recognition and target lookup; capture, IPC and input injection excluded",
        "target": results.unwrap().find("Save").unwrap().text});
    println!("{result}");
    fs::create_dir_all("target").unwrap();
    fs::write(
        "target/scenarios.json",
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
}
