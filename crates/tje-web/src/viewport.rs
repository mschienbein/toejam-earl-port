use jgenesis_common::frontend::FrameSize;

/// Physical-pixel viewport; integer scaling quantizes vertical source lines,
/// not the fractional horizontal stretch needed for the NTSC pixel shape.
pub(crate) fn viewport(
    source: FrameSize,
    width: u32,
    height: u32,
    native_pixels: bool,
    integer_scale: bool,
) -> [f64; 4] {
    if source.width == 0 || source.height == 0 || width == 0 || height == 0 {
        return [0.0; 4];
    }
    let extended = (400..=512).contains(&source.width) && source.height == 224;
    let pixel_aspect = if native_pixels && !extended {
        let aspect = if source.width < 300 {
            8.0 / 7.0
        } else {
            32.0 / 35.0
        };
        if source.height >= 448 {
            2.0 * aspect
        } else {
            aspect
        }
    } else {
        1.0
    };
    let source_width = f64::from(source.width) * pixel_aspect;
    let source_height = f64::from(source.height);
    let mut scale = (f64::from(width) / source_width).min(f64::from(height) / source_height);
    // Below 1×, fit rather than crop or produce an empty viewport.
    if integer_scale && scale >= 1.0 {
        scale = scale.floor();
    }
    let view_width = source_width * scale;
    let view_height = source_height * scale;
    [
        ((f64::from(width) - view_width) * 0.5).floor(),
        ((f64::from(height) - view_height) * 0.5).floor(),
        view_width,
        view_height,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_native_and_browser_viewport_vectors() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tje-native/tests/data/viewport-vectors.json"
        ))
        .unwrap();
        let vectors = fixture["vectors"].as_array().unwrap();
        assert!(!vectors.is_empty());
        for vector in vectors {
            let number = |field: &str, index: usize| vector[field][index].as_u64().unwrap() as u32;
            let actual = viewport(
                FrameSize {
                    width: number("source", 0),
                    height: number("source", 1),
                },
                number("target", 0),
                number("target", 1),
                vector["pixel_shape"] == "native",
                vector["scale"] == "integer",
            );
            for (index, coordinate) in actual.into_iter().enumerate() {
                let expected = vector["expected"][index].as_f64().unwrap();
                assert!(
                    (coordinate - expected).abs() < 1e-4,
                    "vector {vector}: coordinate {index}: {coordinate} != {expected}"
                );
            }
        }
    }

    #[test]
    fn empty_source_or_surface_has_no_viewport() {
        for (source, target) in [
            ([0, 224], [640, 480]),
            ([320, 0], [640, 480]),
            ([320, 224], [0, 480]),
            ([320, 224], [640, 0]),
        ] {
            assert_eq!(
                viewport(
                    FrameSize {
                        width: source[0],
                        height: source[1]
                    },
                    target[0],
                    target[1],
                    true,
                    true
                ),
                [0.0; 4]
            );
        }
    }
}
