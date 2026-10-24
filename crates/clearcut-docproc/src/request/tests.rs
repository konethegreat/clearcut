// Written by Kone Tshivhinda & Claude
use super::*;

fn n(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap()
}

// ---------- which pages ----------

#[test]
fn a_range_of_pages_runs_from_one_to_another_or_to_the_end() {
    let range = PageRange::new(n(2), Some(n(5))).unwrap();
    assert_eq!((range.first(), range.last()), (n(2), Some(n(5))));
    let open = PageRange::new(n(3), None).unwrap();
    assert_eq!((open.first(), open.last()), (n(3), None));
    assert_eq!(PageRange::single(n(7)).last(), Some(n(7)));
}

#[test]
fn a_range_may_not_end_before_it_starts_or_be_more_than_fifty_pages() {
    assert_eq!(PageRange::MAX_PAGES, 50);
    assert!(PageRange::new(n(5), Some(n(4))).is_err());
    assert!(PageRange::new(n(1), Some(n(50))).is_ok());
    assert!(PageRange::new(n(1), Some(n(51))).is_err());
    assert!(PageRange::new(n(10), Some(n(59))).is_ok());
    assert!(PageRange::new(n(10), Some(n(60))).is_err());
    assert!(PageRange::new(n(1), Some(n(1))).is_ok());
    assert!(PageRange::new(n(u32::MAX), Some(n(u32::MAX))).is_ok());
}

// ---------- pictures ----------

#[test]
fn the_picture_formats_are_png_and_jpeg() {
    assert_eq!(ImageFormat::Png.as_str(), "png");
    assert_eq!(ImageFormat::Jpeg.as_str(), "jpeg");
    assert_eq!(ImageFormat::Png.image_type(), ImageType::Png);
    assert_eq!(ImageFormat::Jpeg.image_type(), ImageType::Jpeg);
    assert_eq!(ImageFormat::of(ImageType::Png), ImageFormat::Png);
    assert_eq!(ImageFormat::of(ImageType::Jpeg), ImageFormat::Jpeg);
}

#[test]
fn drawing_defaults_to_150_dpi_png_and_a_longest_side_of_2000() {
    let defaults = RenderOptions::default();
    assert_eq!(
        (defaults.dpi(), defaults.format(), defaults.max_edge()),
        (150, ImageFormat::Png, 2000)
    );
}

#[test]
fn the_resolution_and_longest_side_are_kept_within_what_the_worker_allows() {
    assert!(RenderOptions::new(100, ImageFormat::Png, 500).is_ok());
    assert!(RenderOptions::new(300, ImageFormat::Jpeg, 2576).is_ok());
    for (dpi, edge) in [(99, 2000), (301, 2000), (150, 499), (150, 2577), (0, 0)] {
        assert!(
            RenderOptions::new(dpi, ImageFormat::Png, edge).is_err(),
            "{dpi} {edge}"
        );
    }
    assert_eq!(MIN_DPI, 100);
    assert_eq!(MAX_DPI, 300);
    assert_eq!((MIN_EDGE, MAX_EDGE), (500, 2576));
}

// ---------- boxes ----------

#[test]
fn a_box_is_a_stretch_of_the_picture_in_thousandths() {
    let b = MaskBox::new(10, 20, 300, 40).unwrap();
    assert_eq!((b.x(), b.y(), b.w(), b.h()), (10, 20, 300, 40));
    assert!(MaskBox::new(0, 0, 1000, 1000).is_ok());
    assert!(MaskBox::new(999, 999, 1, 1).is_ok());
}

#[test]
fn a_box_must_have_a_size_and_stay_inside_the_picture() {
    for (x, y, w, h) in [
        (0, 0, 0, 10),
        (0, 0, 10, 0),
        (995, 0, 10, 10),
        (0, 995, 10, 10),
        (1001, 0, 1, 1),
        (0, 0, 1001, 1),
        (u16::MAX, 0, u16::MAX, 1),
        (u16::MAX, u16::MAX, u16::MAX, u16::MAX),
    ] {
        assert!(MaskBox::new(x, y, w, h).is_err(), "{x} {y} {w} {h}");
    }
}

#[test]
fn at_most_five_hundred_boxes_in_one_request() {
    assert_eq!(MAX_BOXES, 500);
}
