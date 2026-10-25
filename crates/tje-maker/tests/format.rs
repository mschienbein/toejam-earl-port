use tje_maker::*;

const SAMPLES: [&[u8]; 3] = [
    include_bytes!("../../../courses/meadow.tjemaker.json"),
    include_bytes!("../../../courses/two-gardens.tjemaker.json"),
    include_bytes!("../../../courses/vanishing-square.tjemaker.json"),
];
fn sample() -> Course {
    ValidatedCourse::from_json(SAMPLES[0])
        .unwrap()
        .course()
        .clone()
}

#[test]
fn authored_samples_round_trip_json_digest_and_compact_checksummed_codes() {
    for bytes in SAMPLES {
        let course = ValidatedCourse::from_json(bytes).unwrap();
        assert_eq!(
            ValidatedCourse::from_json(course.canonical_json()).unwrap(),
            course
        );
        let code = course.to_share_code().unwrap();
        assert!(code.len() < bytes.len());
        assert!(code.len() < 8192);
        assert_eq!(ValidatedCourse::from_share_code(&code).unwrap(), course);
        assert_eq!(course.to_share_code().unwrap(), code);
    }
}
#[test]
fn canonical_identity_ignores_object_order_key_order_and_unmerged_runs() {
    let a = ValidatedCourse::new(sample()).unwrap();
    let mut changed = sample();
    changed.levels[0].objects.reverse();
    changed.levels[0].spawn_points.reverse();
    changed.levels[0].cells = vec![(1, 30), (20, 30), (473, 30)];
    let b = ValidatedCourse::from_json(&serde_json::to_vec_pretty(&changed).unwrap()).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.digest(), b.digest());
    changed.title.push('!');
    assert_ne!(a.digest(), ValidatedCourse::new(changed).unwrap().digest());
}
#[test]
fn catalog_revision_is_explicit_and_part_of_identity() {
    let a = ValidatedCourse::new(sample()).unwrap();
    let mut changed = sample();
    changed.catalog = Catalog::Rev00;
    let b = ValidatedCourse::new(changed).unwrap();
    assert_ne!(a.digest(), b.digest());
}
fn invalid(mut course: Course, change: impl FnOnce(&mut Course), expected: &str) {
    change(&mut course);
    let error = ValidatedCourse::new(course).unwrap_err().to_string();
    assert!(
        error.contains(expected),
        "{error:?} must contain {expected:?}"
    );
}
#[test]
fn clear_structural_errors_precede_any_machine_mutation() {
    invalid(sample(), |c| c.schema = "tje-maker/2".into(), "schema");
    invalid(
        sample(),
        |c| c.runtime_seed = "18446744073709551616".into(),
        "sixteen lowercase",
    );
    invalid(
        sample(),
        |c| c.runtime_seed = "000000000000ABCD".into(),
        "sixteen lowercase",
    );
    invalid(
        sample(),
        |c| c.title = "bad\u{1b}[31m".into(),
        "control characters",
    );
    invalid(sample(), |c| c.author = "é".repeat(33), "64 bytes");
    invalid(sample(), |c| c.rules.heroes = 0, "rules.heroes");
    invalid(
        sample(),
        |c| c.rules.inventory.push(28),
        "present identities",
    );
    invalid(sample(), |c| c.levels[0].cells[0].0 -= 1, "run total");
    invalid(sample(), |c| c.levels[0].cells[0].0 = 0, "positive");
    for cell in [122, 0x1000, 0x2000, 0x4000, 0x8000] {
        invalid(sample(), |c| c.levels[0].cells[0].1 = cell, "invalid cell");
    }
    invalid(
        sample(),
        |c| c.levels.push(c.levels[0].clone()),
        "duplicate level ID",
    );
    invalid(
        sample(),
        |c| {
            let duplicate = c.levels[0].objects[0].clone();
            c.levels[0].objects.push(duplicate);
        },
        "duplicate course-wide object",
    );
    invalid(sample(), |c| c.start.spawns[0] = 900, "unknown spawn");
    invalid(
        sample(),
        |c| c.start.spawns[1] = c.start.spawns[0],
        "distinct safe spawn",
    );
    invalid(sample(), |c| c.finish.exit = 101, "explicit exit");
    invalid(
        sample(),
        |c| c.finish.required = vec![102],
        "not a collectible",
    );
    invalid(
        sample(),
        |c| c.levels[0].spawn_points[0].position.x = 129,
        "eight world pixels",
    );
    invalid(
        sample(),
        |c| c.levels[0].spawn_points[0].position.x = 120,
        "outside the level",
    );
    invalid(
        sample(),
        |c| {
            c.levels[0].fall_target = FallTarget::Level {
                level: 99,
                spawn: 1,
            }
        },
        "unknown level",
    );
    invalid(
        sample(),
        |c| {
            c.levels[0].objects.push(Object::Elevator {
                id: 999,
                position: Point { x: 800, y: 800 },
            })
        },
        "explicit link",
    );
}
#[test]
fn path_ownership_reciprocity_and_link_destinations_are_validated() {
    let course = ValidatedCourse::from_json(SAMPLES[2])
        .unwrap()
        .course()
        .clone();
    invalid(
        course.clone(),
        |c| c.levels[0].paths[0].cells[0].directions = 1,
        "reciprocal cardinal",
    );
    invalid(
        course.clone(),
        |c| {
            let duplicate = c.levels[0].paths[0].cells[0];
            c.levels[0].paths[0].cells.push(duplicate);
        },
        "duplicate ownership",
    );
    invalid(
        course,
        |c| c.levels[0].paths[0].replacement = 0xffff,
        "invalid path",
    );
    let course = ValidatedCourse::from_json(SAMPLES[1])
        .unwrap()
        .course()
        .clone();
    invalid(
        course.clone(),
        |c| c.levels[0].links[0].destination.spawn = 99,
        "unknown spawn",
    );
    invalid(
        course,
        |c| c.levels[1].links[0].two_way = false,
        "reciprocal two_way",
    );
}
#[test]
fn strict_json_rejects_unknown_duplicate_float_binary_and_oversized_input() {
    let bytes = std::str::from_utf8(SAMPLES[0]).unwrap();
    for input in [
        bytes.replacen("{", "{\"extra\":0,", 1),
        bytes.replacen("{", "{\"schema\":\"tje-maker/1\",", 1),
        bytes.replace("\"heroes\": 2", "\"heroes\": 2.0"),
        bytes.replace("\"heroes\": 2", "\"heroes\": -1"),
        bytes.replace("\"heroes\": 2", "\"heroes\": 256"),
        bytes.replace("\"tick_limit\": null,", ""),
        bytes.replace("\"earth\"", "\"file:///tmp/terrain\""),
        bytes.replace("\"tje91-rev02/1\"", "\"tje91-rev99/1\""),
        format!("{}{}{}", "[".repeat(17), "0", "]".repeat(17)),
    ] {
        assert!(
            ValidatedCourse::from_json(input.as_bytes()).is_err(),
            "{input}"
        );
    }
    assert!(ValidatedCourse::from_json(&[0xff]).is_err());
    assert!(ValidatedCourse::from_json(&vec![b' '; MAX_BODY_BYTES + 1]).is_err());
    let huge = bytes.replace(
        "\"inventory\": [7]",
        &format!("\"inventory\": [{}]", vec!["0"; 17].join(",")),
    );
    assert!(
        ValidatedCourse::from_json(huge.as_bytes())
            .unwrap_err()
            .to_string()
            .contains("16 entries")
    );
}
#[test]
fn share_codes_reject_corruption_padding_truncation_trailing_data_and_size_bombs() {
    let code = ValidatedCourse::new(sample())
        .unwrap()
        .to_share_code()
        .unwrap();
    for bad in [
        String::new(),
        code.replace("TJM1:", "TJM2:"),
        format!("{code}="),
        format!("{code}\n"),
        format!("{code}AAAA"),
        code[..code.len() - 3].to_string(),
        "TJM1:".to_string() + &"A".repeat(MAX_CODE_BYTES),
    ] {
        assert!(ValidatedCourse::from_share_code(&bad).is_err());
    }
    // SHA-256 protects the canonical body, not unused DEFLATE padding bits.
    // Every mutation must reject or decode to the identical validated course;
    // valid alternative compressed representations are explicitly permitted.
    for index in 5..code.len() {
        let mut bad = code.as_bytes().to_vec();
        bad[index] = if bad[index] == b'A' { b'B' } else { b'A' };
        if let Ok(decoded) = ValidatedCourse::from_share_code(std::str::from_utf8(&bad).unwrap()) {
            assert_eq!(
                decoded,
                ValidatedCourse::new(sample()).unwrap(),
                "changed course accepted at {index}"
            );
            assert!(index > 5 + 48 * 4 / 3, "corrupt header accepted");
        }
    }
}

#[test]
fn snapshot_configuration_decodes_only_valid_canonical_bounded_documents() {
    let document = ValidatedCourse::new(sample()).unwrap();
    let config = bincode::config::standard().with_limit::<{ MAX_BODY_BYTES + 16 }>();
    let bytes = bincode::encode_to_vec(&document, config).unwrap();
    let (decoded, used): (ValidatedCourse, _) = bincode::decode_from_slice(&bytes, config).unwrap();
    assert_eq!(used, bytes.len());
    assert_eq!(decoded, document);
    let oversized_length = bincode::encode_to_vec(MAX_BODY_BYTES + 1, config).unwrap();
    assert!(bincode::decode_from_slice::<ValidatedCourse, _>(&oversized_length, config).is_err());
    for body in [
        b"{\"schema\":\"bad\"}".to_vec(),
        serde_json::to_vec_pretty(document.course()).unwrap(),
    ] {
        let encoded = bincode::encode_to_vec(body, config).unwrap();
        assert!(bincode::decode_from_slice::<ValidatedCourse, _>(&encoded, config).is_err());
    }
}
