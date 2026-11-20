//! Tray icon rendering: fitting, condensed digits and the icon cache.

use lamzu_battery::icons::{IconRenderer, Rgba, alpha_bbox, uses_condensed};

const BLACK: Rgba = Rgba([0, 0, 0, 255]);

#[test]
fn multi_digit_values_use_the_condensed_face() {
    assert!(uses_condensed("95"));
    assert!(uses_condensed("100"));
    assert!(!uses_condensed("7"));
    assert!(!uses_condensed("?"));
}

#[test]
fn rendered_icon_fits_the_canvas() {
    let mut renderer = IconRenderer::new();
    let rgba = renderer.render("100", BLACK);
    assert_eq!(
        rgba.len(),
        (lamzu_battery::ICON_SIZE * lamzu_battery::ICON_SIZE * 4) as usize
    );
    let bbox = alpha_bbox(&rgba, lamzu_battery::ICON_SIZE).expect("digits are drawn");
    assert!(bbox.0 < bbox.2 && bbox.1 < bbox.3);
    assert!(bbox.2 <= lamzu_battery::ICON_SIZE);
    assert!(bbox.3 <= lamzu_battery::ICON_SIZE);
}

#[test]
fn two_digit_icon_uses_the_icon_height() {
    let mut renderer = IconRenderer::new();
    let rgba = renderer.render("95", BLACK);
    let bbox = alpha_bbox(&rgba, lamzu_battery::ICON_SIZE).expect("digits are drawn");
    let height = bbox.3 - bbox.1;
    assert!(
        height >= 45,
        "two-digit digits should fill the icon, got {height}px"
    );
}

#[test]
fn question_mark_is_rendered() {
    let mut renderer = IconRenderer::new();
    let rgba = renderer.render("?", BLACK);
    assert!(alpha_bbox(&rgba, lamzu_battery::ICON_SIZE).is_some());
}

#[test]
fn rendering_is_cached() {
    let mut renderer = IconRenderer::new();
    let first = renderer.render("42", BLACK);
    let second = renderer.render("42", BLACK);
    assert_eq!(first, second);
}

#[test]
fn colour_is_applied_to_every_pixel() {
    let mut renderer = IconRenderer::new();
    let colour = Rgba([10, 20, 30, 255]);
    let rgba = renderer.render("7", colour);
    let mut painted = 0;
    for pixel in rgba.as_chunks::<4>().0 {
        if pixel[3] > 0 {
            assert_eq!(&pixel[..3], &[10, 20, 30]);
            painted += 1;
        }
    }
    assert!(
        painted > 100,
        "expected a visible digit, painted {painted} pixels"
    );
}

#[test]
fn transparent_background() {
    let mut renderer = IconRenderer::new();
    let rgba = renderer.render("42", BLACK);
    assert_eq!(rgba[3], 0, "the corner must stay transparent");
}
