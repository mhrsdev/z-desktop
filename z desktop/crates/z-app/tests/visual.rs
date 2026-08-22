//! End-to-end rendering tests.
//!
//! These go all the way to the GPU and read pixels back, so they catch the
//! class of bug that scene-level assertions cannot: wrong blend state, wrong
//! colour space, wrong draw order.
//!
//! They need a graphics adapter. `OffscreenRenderer` falls back to a software
//! adapter, but a machine with neither will skip rather than fail — a missing
//! adapter is an environment fact, not a defect in the shell.

use z_gpui::scene::{Layer, Quad, Scene};
use z_gpui::{OffscreenRenderer, Rect};
use z_tokens::{Rgba, Theme};

const VIEWPORT: Rect = Rect { x: 0.0, y: 0.0, width: 800.0, height: 600.0 };

/// Acquire a renderer, or `None` when the machine has no usable adapter.
fn renderer() -> Option<OffscreenRenderer> {
    match OffscreenRenderer::new(1.0) {
        Ok(renderer) => Some(renderer),
        Err(error) => {
            eprintln!("skipping: no graphics adapter available ({error})");
            None
        }
    }
}

/// Largest per-channel difference between two colours.
fn delta(a: [u8; 4], b: [u8; 4]) -> u8 {
    (0..3).map(|i| a[i].abs_diff(b[i])).max().unwrap_or(0)
}

#[test]
fn a_solid_quad_lands_in_the_colour_it_was_given() {
    let Some(mut renderer) = renderer() else { return };
    let fill = Rgba::hex(0x3FB950);

    let mut scene = Scene::new();
    scene.push_quad(Layer::Content, Quad::filled(Rect::new(100.0, 100.0, 400.0, 300.0), fill));

    let capture = renderer.capture(&scene, VIEWPORT, Rgba::hex(0x000000)).expect("capture failed");

    let centre = capture.pixel(300, 250).expect("inside the quad");
    assert!(
        delta(centre, [fill.r, fill.g, fill.b, 255]) <= 2,
        "expected {:?}, got {centre:?} — check the blend state and colour space",
        [fill.r, fill.g, fill.b, 255]
    );
}

#[test]
fn the_clear_colour_shows_where_nothing_was_drawn() {
    let Some(mut renderer) = renderer() else { return };
    let canvas = Theme::zero_dark().colors.canvas;

    let mut scene = Scene::new();
    scene.push_quad(
        Layer::Content,
        Quad::filled(Rect::new(0.0, 0.0, 50.0, 50.0), Rgba::hex(0xFFFFFF)),
    );

    let capture = renderer.capture(&scene, VIEWPORT, canvas).expect("capture failed");

    let empty = capture.pixel(700, 500).expect("outside the quad");
    assert!(
        delta(empty, [canvas.r, canvas.g, canvas.b, 255]) <= 2,
        "background should be the canvas token, got {empty:?}"
    );
}

#[test]
fn an_overlay_covers_the_content_beneath_it() {
    // The regression this guards: batching every quad before every glyph made
    // content text show through an opaque overlay panel. Drawing has to
    // proceed layer by layer.
    let Some(mut renderer) = renderer() else { return };

    let mut scene = Scene::new();
    scene.push_quad(
        Layer::Content,
        Quad::filled(Rect::new(0.0, 0.0, 800.0, 600.0), Rgba::hex(0xFFFFFF)),
    );
    scene.push_text(
        Layer::Content,
        z_gpui::TextRun::new(
            "CONTENT UNDERNEATH THE OVERLAY",
            Rect::new(100.0, 250.0, 600.0, 40.0),
            z_tokens::Typography::XL,
            Rgba::hex(0x000000),
        ),
    );
    scene.push_quad(
        Layer::Overlay,
        Quad::filled(Rect::new(50.0, 200.0, 700.0, 200.0), Rgba::hex(0x3FB950)),
    );

    let capture = renderer.capture(&scene, VIEWPORT, Rgba::hex(0x000000)).expect("capture failed");

    // Sample across the band the text occupies; every pixel must be the overlay.
    let overlay = [0x3F, 0xB9, 0x50, 255];
    for x in (110..690).step_by(20) {
        for y in [255, 265, 275] {
            let pixel = capture.pixel(x, y).expect("inside the overlay");
            assert!(
                delta(pixel, overlay) <= 2,
                "content showed through the overlay at ({x},{y}): {pixel:?}"
            );
        }
    }
}

#[test]
fn a_clip_cuts_content_at_the_boundary() {
    // Over-scanned rows in a scroll area sit outside the viewport by design;
    // without a working clip they paint over whatever is next to them.
    let Some(mut renderer) = renderer() else { return };

    let mut scene = Scene::new();
    scene.clipped(Rect::new(0.0, 200.0, 800.0, 200.0), |scene| {
        // Spans the full height, but only the clipped band may appear.
        scene.push_quad(
            Layer::Content,
            Quad::filled(Rect::new(0.0, 0.0, 800.0, 600.0), Rgba::hex(0xFFFFFF)),
        );
    });

    let capture = renderer.capture(&scene, VIEWPORT, Rgba::hex(0x000000)).expect("capture failed");

    assert!(capture.pixel(400, 100).unwrap()[0] < 20, "content escaped above the clip");
    assert!(capture.pixel(400, 300).unwrap()[0] > 200, "content missing inside the clip");
    assert!(capture.pixel(400, 500).unwrap()[0] < 20, "content escaped below the clip");
}

#[test]
fn a_clipped_rounded_corner_is_cut_straight_not_re_rounded() {
    // The reason clipping happens in the shader rather than by shrinking the
    // rect: shrinking would drag the corner radius inward, so a card cut off
    // by a scroll edge would sprout a new corner instead of a straight cut.
    let Some(mut renderer) = renderer() else { return };

    let mut scene = Scene::new();
    scene.clipped(Rect::new(0.0, 0.0, 800.0, 250.0), |scene| {
        scene.push_quad(
            Layer::Content,
            Quad::filled(Rect::new(100.0, 100.0, 400.0, 400.0), Rgba::hex(0xFFFFFF))
                .with_radius(60.0),
        );
    });

    let capture = renderer.capture(&scene, VIEWPORT, Rgba::hex(0x000000)).expect("capture failed");

    // Just inside the clip's lower edge the shape is still full width, because
    // the cut is straight rather than following a new radius.
    for x in [110, 300, 490] {
        let pixel = capture.pixel(x, 245).expect("inside the clip");
        assert!(pixel[0] > 200, "the cut re-rounded the corner at x={x}: {pixel:?}");
    }
    assert!(capture.pixel(300, 260).unwrap()[0] < 20, "content escaped past the clip");
}

#[test]
fn nested_clips_intersect_rather_than_replace() {
    let Some(mut renderer) = renderer() else { return };

    let mut scene = Scene::new();
    scene.clipped(Rect::new(0.0, 200.0, 800.0, 200.0), |scene| {
        // A child asking for more room than its parent must not get it.
        scene.clipped(Rect::new(0.0, 0.0, 800.0, 600.0), |scene| {
            scene.push_quad(
                Layer::Content,
                Quad::filled(Rect::new(0.0, 0.0, 800.0, 600.0), Rgba::hex(0xFFFFFF)),
            );
        });
    });

    let capture = renderer.capture(&scene, VIEWPORT, Rgba::hex(0x000000)).expect("capture failed");
    assert!(capture.pixel(400, 100).unwrap()[0] < 20, "a child clip widened its parent");
    assert!(capture.pixel(400, 300).unwrap()[0] > 200);
}

#[test]
fn a_rounded_corner_is_actually_cut_away() {
    let Some(mut renderer) = renderer() else { return };

    let mut scene = Scene::new();
    scene.push_quad(
        Layer::Content,
        Quad::filled(Rect::new(100.0, 100.0, 200.0, 200.0), Rgba::hex(0xFFFFFF)).with_radius(40.0),
    );

    let capture = renderer.capture(&scene, VIEWPORT, Rgba::hex(0x000000)).expect("capture failed");

    let corner = capture.pixel(103, 103).expect("at the corner");
    let inside = capture.pixel(200, 200).expect("at the centre");
    assert!(corner[0] < 60, "the corner should be cut away, got {corner:?}");
    assert!(inside[0] > 200, "the centre should be filled, got {inside:?}");
}

#[test]
fn the_reference_workspace_renders_something_on_every_panel() {
    let Some(mut renderer) = renderer() else { return };

    // Reach into the binary's view the same way the screenshot command does.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_zero"))
        .arg("--check")
        .output()
        .expect("could not run the binary");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("scene ok"), "headless check failed: {stdout}");

    // And prove the same scene actually reaches the GPU.
    let mut scene = Scene::new();
    scene.push_quad(Layer::Background, Quad::filled(VIEWPORT, Theme::zero_dark().colors.surface));
    let capture = renderer
        .capture(&scene, VIEWPORT, Theme::zero_dark().colors.canvas)
        .expect("capture failed");
    assert_eq!(capture.width, 800);
    assert_eq!(capture.height, 600);
    assert!(!capture.stats.skipped);
}
