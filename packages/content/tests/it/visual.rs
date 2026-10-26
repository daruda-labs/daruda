use daruda_content::visual::{RasterImage, decode_image, decode_image_bounded};

fn encode_png(width: u32, height: u32, px: [u8; 4]) -> Vec<u8> {
    let mut img = image::RgbaImage::new(width, height);
    for p in img.pixels_mut() {
        *p = image::Rgba(px);
    }
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)
        .expect("encode png");
    buf.into_inner()
}

#[test]
fn logical_size_divides_pixels_by_scale() {
    let hidpi = RasterImage {
        width: 40,
        height: 80,
        bgra: vec![],
        scale: 2.0,
    };
    assert_eq!(hidpi.logical_size(), (20.0, 40.0));

    let native = RasterImage {
        width: 30,
        height: 30,
        bgra: vec![],
        scale: 1.0,
    };
    assert_eq!(native.logical_size(), (30.0, 30.0));
}

#[test]
fn decodes_png_to_expected_dimensions() {
    let png = encode_png(3, 2, [10, 20, 30, 255]);
    let img = decode_image(&png).expect("decode should succeed");
    assert_eq!((img.width, img.height), (3, 2));
    assert_eq!(img.bgra.len(), (3 * 2 * 4) as usize);
    // Encoded RGBA 10,20,30 must come back as BGRA.
    assert_eq!(&img.bgra[..4], &[30, 20, 10, 255]);
}

#[test]
fn bounded_decode_does_not_narrow_the_existing_decoder() {
    let png = encode_png(3, 2, [10, 20, 30, 255]);
    assert!(decode_image_bounded(&png, 2, 1024).is_err());
    assert_eq!(decode_image(&png).unwrap().width, 3);
}
