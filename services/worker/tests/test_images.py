# Written by Kone Tshivhinda & Claude
"""Opening a picture without trusting it, and sending one back with only pixels (ADR-0016)."""

from __future__ import annotations

import io
import time

import pytest
from PIL import Image

from clearcut_worker.images import (
    DEFAULT_MAX_EDGE,
    MAX_IMAGE_BYTES,
    MAX_IMAGE_PIXELS,
    EncodedImage,
    ImageTooLarge,
    encode_image,
    open_image,
    sniff,
)
from clearcut_worker.inspect import NotAPdf, Unreadable

from .pictures import (
    COMMENT_MARKER,
    EXIF_MARKER,
    GPS_MARKER,
    claims_to_be,
    jpeg,
    noise,
    png,
    png_chunk_types,
    png_of,
    size_of,
)

# ---------- what kind of file it is, judged by the bytes ----------


def test_the_kind_of_a_file_comes_from_its_first_bytes() -> None:
    assert sniff(b"%PDF-1.7\n...") == "pdf"
    assert sniff(png()) == "png"
    assert sniff(jpeg()) == "jpeg"


@pytest.mark.parametrize(
    "data",
    [
        b"",
        b"<html>nope</html>",
        b"GIF89a\x01\x00\x01\x00",
        b"RIFF\x24\x00\x00\x00WEBPVP8 ",
        b"II*\x00\x08\x00\x00\x00",  # TIFF
        b"\x89PNG",  # a PNG signature cut short
        b"\xff\xd8",  # a JPEG signature cut short
        b" %PDF-1.4",  # not at the start
    ],
)
def test_anything_else_is_not_a_kind_the_worker_reads(data: bytes) -> None:
    assert sniff(data) is None


# ---------- opening ----------


def test_a_png_opens_as_an_rgb_picture_of_the_same_size() -> None:
    image = open_image(png(30, 20, (10, 20, 30)))
    assert image.mode == "RGB"
    assert image.size == (30, 20)
    assert image.getpixel((0, 0)) == (10, 20, 30)


@pytest.mark.parametrize(
    ("mode", "colour"),
    [("L", 128), ("P", 3), ("1", 1), ("RGBA", (10, 20, 30, 255)), ("LA", (100, 255))],
)
def test_every_png_mode_comes_out_as_rgb(mode: str, colour: tuple[int, ...] | int) -> None:
    assert open_image(png(8, 8, colour, mode)).mode == "RGB"


def test_transparent_pixels_become_white_not_black() -> None:
    image = open_image(png(4, 4, (0, 0, 0, 0), "RGBA"))
    assert image.getpixel((1, 1)) == (255, 255, 255)


def test_a_half_transparent_pixel_is_mixed_with_white() -> None:
    red, green, blue = open_image(png(4, 4, (0, 0, 0, 128), "RGBA")).getpixel((1, 1))  # type: ignore[misc]
    assert 120 <= red == green == blue <= 135


def test_a_jpeg_opens_and_a_cmyk_one_comes_out_as_rgb() -> None:
    assert open_image(jpeg(16, 12)).size == (16, 12)
    assert open_image(jpeg(16, 12, (0, 0, 0, 0), "CMYK")).mode == "RGB"


@pytest.mark.parametrize("orientation", [1, 2, 3, 4, 5, 6, 7, 8])
def test_a_photo_is_turned_upright_from_its_exif_orientation(orientation: int) -> None:
    image = open_image(jpeg(200, 100, orientation=orientation))
    # Orientations 5 to 8 are quarter turns, so the sides swap.
    assert image.size == ((100, 200) if orientation >= 5 else (200, 100))


def test_an_animated_png_is_read_as_its_first_picture_only() -> None:
    first = Image.new("RGB", (12, 8), (255, 0, 0))
    second = Image.new("RGB", (12, 8), (0, 0, 255))
    out = io.BytesIO()
    first.save(out, "PNG", save_all=True, append_images=[second], duration=100)
    image = open_image(out.getvalue())
    assert image.size == (12, 8)
    assert image.getpixel((0, 0)) == (255, 0, 0)


def test_a_picture_is_a_new_object_with_no_metadata_carried_over() -> None:
    image = open_image(jpeg(20, 10, with_metadata=True))
    assert not image.info
    assert not image.getexif()


# ---------- refusing ----------


def test_other_kinds_of_file_are_not_supported() -> None:
    for data in (b"%PDF-1.4\n", b"GIF89a....", b"<html>"):
        with pytest.raises(NotAPdf):
            open_image(data)


@pytest.mark.parametrize(
    "data",
    [
        png(40, 40)[:60],  # cut short
        png_of(noise(40, 40))[:3000],  # the picture data stops half way
        b"\x89PNG\r\n\x1a\n" + b"not chunks at all",
        jpeg(40, 40)[:50],
        b"\xff\xd8\xff" + b"garbage" * 5,
    ],
)
def test_a_damaged_picture_is_unreadable(data: bytes) -> None:
    with pytest.raises(Unreadable):
        open_image(data)


def test_a_picture_that_claims_to_be_huge_is_refused_before_anything_is_decoded() -> None:
    started = time.monotonic()
    with pytest.raises(ImageTooLarge):
        open_image(claims_to_be(30_000, 30_000))
    assert time.monotonic() - started < 2


def test_the_pixel_limit_is_exact() -> None:
    assert MAX_IMAGE_PIXELS == 40_000_000
    assert open_image(png(10, 10), max_pixels=100).size == (10, 10)
    with pytest.raises(ImageTooLarge):
        open_image(png(11, 10), max_pixels=100)
    with pytest.raises(ImageTooLarge):
        open_image(claims_to_be(8001, 5000))


def test_a_picture_with_no_area_is_unreadable_not_a_crash() -> None:
    with pytest.raises(Unreadable):
        open_image(claims_to_be(0, 10))


def test_the_messages_never_contain_picture_content() -> None:
    for data in (png(10, 10, text="SECRET-NOTE")[:-30], b"\x89PNG\r\n\x1a\nSECRET"):
        with pytest.raises(Unreadable) as caught:
            open_image(data)
        assert "SECRET" not in str(caught.value)


# ---------- sending a picture back ----------


def test_a_png_comes_back_as_a_png_of_the_same_pixels() -> None:
    original = noise(30, 20)
    result = encode_image(original)
    assert isinstance(result, EncodedImage)
    assert result.media_type == "image/png"
    assert (result.width, result.height) == (30, 20)
    with Image.open(io.BytesIO(result.data)) as again:
        assert again.tobytes() == original.tobytes()


def test_a_png_carries_only_the_chunks_a_picture_needs() -> None:
    assert set(png_chunk_types(encode_image(noise(30, 20)).data)) <= {"IHDR", "IDAT", "IEND"}


@pytest.mark.parametrize("fmt", ["png", "jpeg"])
def test_no_metadata_survives_the_trip(fmt: str) -> None:
    source = open_image(jpeg(60, 40, with_metadata=True))
    data = encode_image(source, fmt=fmt).data
    assert EXIF_MARKER.encode() not in data
    assert GPS_MARKER.encode() not in data
    assert COMMENT_MARKER not in data
    assert b"Exif" not in data
    assert b"acsp" not in data  # an ICC colour profile has this signature
    assert b"ICC_PROFILE" not in data


def test_a_text_chunk_in_a_png_does_not_survive_either() -> None:
    data = encode_image(open_image(png(20, 10, text="SECRET-NOTE"))).data
    assert b"SECRET-NOTE" not in data
    assert b"tEXt" not in data


def test_the_same_picture_gives_the_same_bytes() -> None:
    picture = noise(40, 30)
    assert encode_image(picture).data == encode_image(picture).data
    assert encode_image(picture, fmt="jpeg").data == encode_image(picture, fmt="jpeg").data


def test_a_jpeg_is_made_when_asked_for() -> None:
    result = encode_image(noise(30, 20), fmt="jpeg")
    assert result.media_type == "image/jpeg"
    assert result.data.startswith(b"\xff\xd8\xff")
    assert result.data.endswith(b"\xff\xd9")
    assert size_of(result.data) == (30, 20)


def test_a_long_edge_over_the_cap_is_scaled_down_keeping_the_shape() -> None:
    result = encode_image(Image.new("RGB", (3000, 1500), (255, 255, 255)), max_edge=2000)
    assert (result.width, result.height) == (2000, 1000)
    assert size_of(result.data) == (2000, 1000)
    tall = encode_image(Image.new("RGB", (1500, 3001), (255, 255, 255)), max_edge=2000)
    assert tall.height == 2000
    assert abs(tall.width - 1500 * 2000 / 3001) <= 1


def test_a_picture_within_the_cap_is_never_enlarged_or_changed_in_size() -> None:
    result = encode_image(Image.new("RGB", (2000, 700), (255, 255, 255)), max_edge=2000)
    assert (result.width, result.height) == (2000, 700)
    assert DEFAULT_MAX_EDGE == 2000
    small = encode_image(Image.new("RGB", (300, 200), (255, 255, 255)), max_edge=2000)
    assert (small.width, small.height) == (300, 200)


def test_a_png_over_the_byte_limit_is_sent_as_a_jpeg_of_the_same_size() -> None:
    # Faint noise is far smaller as a JPEG than as a PNG, so a limit between the two
    # (worked out here, not guessed) forces the fallback and nothing more.
    picture = noise(300, 300, amplitude=6)
    as_png = len(encode_image(picture, max_bytes=10**9).data)
    as_jpeg = len(encode_image(picture, fmt="jpeg", max_bytes=10**9).data)
    assert as_jpeg < as_png
    limit = (as_png + as_jpeg) // 2
    result = encode_image(picture, max_bytes=limit)
    assert result.media_type == "image/jpeg"
    assert len(result.data) <= limit
    assert (result.width, result.height) == (300, 300)


def test_a_picture_that_still_does_not_fit_is_scaled_down_until_it_does() -> None:
    picture = noise(400, 400)
    result = encode_image(picture, max_bytes=30_000)
    assert len(result.data) <= 30_000
    assert result.width < 400
    assert abs(result.width - result.height) <= 1
    assert size_of(result.data) == (result.width, result.height)


def test_a_picture_that_cannot_be_made_small_enough_is_refused() -> None:
    with pytest.raises(ImageTooLarge):
        encode_image(noise(400, 400), max_bytes=200)


def test_the_default_byte_limit_is_five_mebibytes() -> None:
    assert MAX_IMAGE_BYTES == 5 * 1024 * 1024


def test_png_of_a_helper_round_trips_for_the_other_tests() -> None:
    assert sniff(png_of(noise(5, 5))) == "png"


def test_a_palette_picture_with_a_transparent_colour_becomes_white_there() -> None:
    palette = Image.new("P", (4, 4), 0)
    palette.putpalette([0, 0, 0, 255, 0, 0] + [0, 0, 0] * 254)
    palette.info["transparency"] = 0
    out = io.BytesIO()
    palette.save(out, "PNG", transparency=0)
    assert open_image(out.getvalue()).getpixel((1, 1)) == (255, 255, 255)


def test_metadata_on_a_picture_handed_straight_to_the_encoder_is_not_sent() -> None:
    from PIL import ImageCms

    profile = ImageCms.ImageCmsProfile(ImageCms.createProfile("sRGB")).tobytes()
    picture = noise(30, 20)
    picture.info = {"icc_profile": profile, "comment": COMMENT_MARKER, "dpi": (72, 72)}
    for fmt in ("png", "jpeg"):
        data = encode_image(picture, fmt=fmt).data
        assert b"acsp" not in data
        assert COMMENT_MARKER not in data
        assert b"pHYs" not in data
        picture.info = {"icc_profile": profile, "comment": COMMENT_MARKER, "dpi": (72, 72)}


def test_an_image_that_is_not_rgb_is_converted_before_it_is_encoded() -> None:
    result = encode_image(Image.new("L", (10, 10), 128))
    with Image.open(io.BytesIO(result.data)) as again:
        assert again.mode == "RGB"


def test_a_png_exactly_at_the_byte_limit_is_kept_as_a_png() -> None:
    picture = noise(40, 40)
    size = len(encode_image(picture).data)
    assert encode_image(picture, max_bytes=size).media_type == "image/png"
    assert encode_image(picture, max_bytes=size - 1).media_type == "image/jpeg"


def test_the_smallest_a_picture_is_made_is_a_long_edge_of_one_hundred() -> None:
    # Noise that is only just too big at 100 pixels is refused; a limit it can meet is not.
    sample = noise(100, 100)
    small = len(encode_image(sample, fmt="jpeg", max_bytes=10**9).data)
    assert encode_image(sample, fmt="jpeg", max_bytes=small).width == 100
    with pytest.raises(ImageTooLarge):
        encode_image(sample, fmt="jpeg", max_bytes=small // 2)


def test_a_jpeg_keeps_fine_detail_and_colour_edges() -> None:
    # One-pixel red and blue columns: strong colour changes that heavy compression or
    # colour sub-sampling would smear into each other.
    stripes = Image.new("RGB", (32, 8))
    for x in range(32):
        for y in range(8):
            stripes.putpixel((x, y), (255, 0, 0) if x % 2 == 0 else (0, 0, 255))
    result = encode_image(stripes, fmt="jpeg")
    with Image.open(io.BytesIO(result.data)) as again:
        red = again.convert("RGB").getpixel((10, 4))
        blue = again.convert("RGB").getpixel((11, 4))
    assert isinstance(red, tuple)
    assert isinstance(blue, tuple)
    assert red[0] > 200
    assert red[2] < 60
    assert blue[2] > 200
    assert blue[0] < 60
