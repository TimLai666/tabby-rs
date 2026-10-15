use crate::windows_instance_message::{decode_payload, encode_payload, WireFormat};

fn args_of(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_string()).collect()
}

fn has_restart_guidance(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("restart")
        || lower.contains("reopen")
        || lower.contains("relaunch")
        || error.contains("重新啟動")
        || error.contains("重開")
}

#[test]
fn json_preserves_cwd_and_every_argument_verbatim() {
    let cwd = "C:\\proj 資料\\😀";
    let args = args_of(&[
        "plain",
        "a|b",
        "||",
        "|leading",
        "trailing|",
        "two  spaces",
        "single'quote",
        "double\"quote",
        "back\\slash\\\\end",
        "tab\tcr\rlf\n",
        "中文台灣",
        "emoji😀🎉",
        "",
        "between",
        "tail",
        "",
    ]);
    assert_eq!(
        args.last().map(String::as_str),
        Some(""),
        "fixture must end with a genuine trailing empty argument"
    );

    let bytes = encode_payload(cwd, &args, WireFormat::Json).expect("json encode");
    let (decoded_args, decoded_cwd) =
        decode_payload(&bytes, WireFormat::Json).expect("json decode");

    assert_eq!(decoded_cwd, cwd);
    assert_eq!(decoded_args, args);
}

#[test]
fn json_preserves_empty_argument_array() {
    let cwd = "C:\\wd";
    let bytes = encode_payload(cwd, &[], WireFormat::Json).expect("json encode");
    let (decoded_args, decoded_cwd) =
        decode_payload(&bytes, WireFormat::Json).expect("json decode");

    assert_eq!(decoded_cwd, cwd);
    assert!(decoded_args.is_empty());
}

#[test]
fn json_preserves_empty_cwd() {
    let args = args_of(&["a|b", "", "trailing|"]);
    let bytes = encode_payload("", &args, WireFormat::Json).expect("json encode");
    let (decoded_args, decoded_cwd) =
        decode_payload(&bytes, WireFormat::Json).expect("json decode");

    assert_eq!(decoded_cwd, "");
    assert_eq!(decoded_args, args);
}

#[test]
fn json_decodes_canonical_tuple() {
    assert_eq!(
        decode_payload(b"[\"cwd\",[]]", WireFormat::Json),
        Some((Vec::new(), "cwd".to_string()))
    );
    assert_eq!(
        decode_payload(b"[\"cwd\",[\"a\",\"b\"]]", WireFormat::Json),
        Some((args_of(&["a", "b"]), "cwd".to_string()))
    );
}

#[test]
fn json_encode_rejects_nul() {
    assert!(encode_payload("c\0wd", &[], WireFormat::Json).is_err());
    assert!(encode_payload("cwd", &args_of(&["a\0b"]), WireFormat::Json).is_err());
}

#[test]
fn json_decode_rejects_empty_input() {
    assert!(decode_payload(b"", WireFormat::Json).is_none());
    assert!(decode_payload(b"   ", WireFormat::Json).is_none());
}

#[test]
fn json_decode_rejects_truncated_input() {
    let bytes =
        encode_payload("C:\\wd", &args_of(&["a|b", "x"]), WireFormat::Json).expect("json encode");
    assert!(bytes.len() > 2);

    for cut in [0usize, 1, bytes.len() / 2, bytes.len() - 1] {
        assert!(
            decode_payload(&bytes[..cut], WireFormat::Json).is_none(),
            "truncated to {cut} bytes must be rejected"
        );
    }
    assert!(decode_payload(b"[\"cwd\"", WireFormat::Json).is_none());
}

#[test]
fn json_decode_rejects_garbage() {
    assert!(decode_payload(b"not json", WireFormat::Json).is_none());
    assert!(decode_payload(b"{invalid", WireFormat::Json).is_none());
    assert!(decode_payload(b"<xml/>", WireFormat::Json).is_none());
}

#[test]
fn json_decode_rejects_invalid_utf8() {
    assert!(decode_payload(&[0xff, 0xfe, 0xfd], WireFormat::Json).is_none());
    assert!(decode_payload(b"[\"\xff\xfe\", []]", WireFormat::Json).is_none());
}

#[test]
fn json_decode_rejects_wrong_tuple_length() {
    assert!(decode_payload(b"[]", WireFormat::Json).is_none());
    assert!(decode_payload(b"[\"cwd\"]", WireFormat::Json).is_none());
    assert!(decode_payload(b"[\"cwd\",[],\"extra\"]", WireFormat::Json).is_none());
}

#[test]
fn json_decode_rejects_wrong_types() {
    assert!(decode_payload(b"{}", WireFormat::Json).is_none());
    assert!(decode_payload(b"\"cwd\"", WireFormat::Json).is_none());
    assert!(decode_payload(b"[123,[]]", WireFormat::Json).is_none());
    assert!(decode_payload(b"[\"cwd\",\"args\"]", WireFormat::Json).is_none());
    assert!(decode_payload(b"[\"cwd\",[1,2]]", WireFormat::Json).is_none());
    assert!(decode_payload(b"[\"cwd\",[null]]", WireFormat::Json).is_none());
}

#[test]
fn json_decode_rejects_nul_values() {
    assert!(decode_payload(b"[\"c\\u0000wd\",[]]", WireFormat::Json).is_none());
    assert!(decode_payload(b"[\"cwd\",[\"a\\u0000b\"]]", WireFormat::Json).is_none());
}

#[test]
fn legacy_round_trips_representable_arguments() {
    let cwd = "C:\\Users\\me\\proj 資料";
    let args = args_of(&[
        "C:\\Windows\\app.exe",
        "--new-window",
        "C:\\tmp\\a b.txt",
        "single'quote",
        "double\"quote",
        "back\\slash",
        "tab\tcr\rlf\n",
        "中文emoji😀",
        "",
        "tail",
    ]);

    let bytes = encode_payload(cwd, &args, WireFormat::Legacy).expect("legacy encode");
    let (decoded_args, decoded_cwd) =
        decode_payload(&bytes, WireFormat::Legacy).expect("legacy decode");

    assert_eq!(decoded_cwd, cwd);
    assert_eq!(decoded_args, args);
}

#[test]
fn legacy_round_trips_empty_argument_array() {
    let bytes = encode_payload("C:\\wd", &[], WireFormat::Legacy).expect("legacy encode");
    let (args, cwd) = decode_payload(&bytes, WireFormat::Legacy).expect("legacy decode");

    assert_eq!(cwd, "C:\\wd");
    assert!(args.is_empty());
}

#[test]
fn legacy_distinguishes_empty_argument_array_from_single_empty_argument() {
    let empty_argv = encode_payload("C:\\wd", &[], WireFormat::Legacy).expect("legacy encode");
    let single_empty =
        encode_payload("C:\\wd", &args_of(&[""]), WireFormat::Legacy).expect("legacy encode");

    let (empty_args, empty_cwd) =
        decode_payload(&empty_argv, WireFormat::Legacy).expect("legacy decode");
    let (single_args, single_cwd) =
        decode_payload(&single_empty, WireFormat::Legacy).expect("legacy decode");

    assert_eq!(empty_cwd, single_cwd);
    assert!(empty_args.is_empty());
    assert_eq!(single_args, args_of(&[""]));
    assert_ne!(empty_args, single_args);
}

#[test]
fn legacy_round_trips_empty_cwd() {
    let args = args_of(&["a", ""]);
    let bytes = encode_payload("", &args, WireFormat::Legacy).expect("legacy encode");
    let (decoded_args, decoded_cwd) =
        decode_payload(&bytes, WireFormat::Legacy).expect("legacy decode");

    assert_eq!(decoded_cwd, "");
    assert_eq!(decoded_args, args);
}

#[test]
fn legacy_encode_rejects_pipe() {
    assert!(encode_payload("C:\\wd", &args_of(&["a|b"]), WireFormat::Legacy).is_err());
    assert!(encode_payload("C:\\wd", &args_of(&["|", "x|y|z"]), WireFormat::Legacy).is_err());
    assert!(encode_payload("C|\\wd", &args_of(&["a"]), WireFormat::Legacy).is_err());
}

#[test]
fn legacy_encode_rejects_nul() {
    assert!(encode_payload("C:\\wd", &args_of(&["a\0b"]), WireFormat::Legacy).is_err());
    assert!(encode_payload("C\0wd", &args_of(&["a"]), WireFormat::Legacy).is_err());
}

#[test]
fn legacy_pipe_rejection_guides_restart_without_leaking_input() {
    let secret_cwd = "C:\\SECRET_CWD_4b7c";
    let secret_arg = "SECRET_TOKEN_9f3a|rest";

    let error = encode_payload(secret_cwd, &args_of(&[secret_arg]), WireFormat::Legacy)
        .expect_err("pipe must be rejected");

    assert!(
        has_restart_guidance(&error),
        "rejection should guide a restart: {error:?}"
    );
    assert!(
        !error.contains("SECRET_TOKEN_9f3a"),
        "leaked argv value: {error:?}"
    );
    assert!(
        !error.contains("SECRET_CWD_4b7c"),
        "leaked cwd value: {error:?}"
    );
}

#[test]
fn legacy_decodes_fixed_original_fixture_bytes() {
    let fixture: &[u8] = b"C:\\Users\\me\\proj|C:\\Windows\\app.exe|--new-window|C:\\tmp\\a b.txt|\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e|\xf0\x9f\x98\x80\x00";

    let (args, cwd) = decode_payload(fixture, WireFormat::Legacy).expect("decode original fixture");

    assert_eq!(cwd, "C:\\Users\\me\\proj");
    assert_eq!(
        args,
        args_of(&[
            "C:\\Windows\\app.exe",
            "--new-window",
            "C:\\tmp\\a b.txt",
            "日本語",
            "😀",
        ])
    );
}

#[test]
fn legacy_decodes_fixed_original_fixture_with_trailing_empty_argument() {
    let fixture: &[u8] = b"C:\\wd|app.exe|\x00";
    let (args, cwd) = decode_payload(fixture, WireFormat::Legacy).expect("decode original fixture");

    assert_eq!(cwd, "C:\\wd");
    assert_eq!(args, args_of(&["app.exe", ""]));
}

#[test]
fn legacy_decode_respects_nul_terminator_and_slice_boundary() {
    let with_trailing: &[u8] = b"C:\\wd|app.exe\x00GARBAGE_NOT_PART_OF_PAYLOAD";
    let (args, cwd) =
        decode_payload(with_trailing, WireFormat::Legacy).expect("decode nul terminated");

    assert_eq!(cwd, "C:\\wd");
    assert_eq!(args, args_of(&["app.exe"]));
}

#[test]
fn legacy_decode_rejects_missing_nul_terminator() {
    assert!(decode_payload(b"", WireFormat::Legacy).is_none());
    assert!(decode_payload(b"C:\\wd", WireFormat::Legacy).is_none());
    assert!(decode_payload(b"C:\\wd|app.exe", WireFormat::Legacy).is_none());
    assert!(decode_payload(b"C:\\a b|app.exe|--flag", WireFormat::Legacy).is_none());
}

#[test]
fn json_decode_rejects_frame_with_trailing_garbage_and_cut_closing_byte() {
    let args = args_of(&["a", ""]);
    let frame = encode_payload("C:\\wd", &args, WireFormat::Json).expect("json encode");
    let mut combined = frame.clone();
    combined.extend_from_slice(b"garbage-after-frame");

    assert!(
        decode_payload(&combined, WireFormat::Json).is_none(),
        "a valid frame plus trailing bytes must be rejected"
    );
    assert_eq!(
        decode_payload(&combined[..frame.len()], WireFormat::Json),
        Some((args.clone(), "C:\\wd".to_string())),
        "the exact valid frame length must be accepted"
    );
    assert!(
        decode_payload(&combined[..frame.len() - 1], WireFormat::Json).is_none(),
        "removing the closing byte must be rejected"
    );
}

#[test]
fn legacy_decode_rejects_nul_that_lies_just_outside_the_slice() {
    let backing: &[u8] = b"C:\\wd|app.exe\x00rest-of-backing-buffer";
    let nul_index = backing
        .iter()
        .position(|byte| *byte == 0)
        .expect("backing buffer carries a NUL");

    assert!(
        decode_payload(&backing[..nul_index], WireFormat::Legacy).is_none(),
        "a slice that ends before the backing NUL must be rejected"
    );
    let (args, cwd) = decode_payload(&backing[..=nul_index], WireFormat::Legacy)
        .expect("a slice that includes the NUL must decode");
    assert_eq!(cwd, "C:\\wd");
    assert_eq!(args, args_of(&["app.exe"]));
}

#[test]
fn legacy_decodes_invalid_utf8_with_original_lossy_replacement() {
    let fixture: &[u8] = b"C:\\wd|\xff|app|\xe2\x82\x00";
    let (args, cwd) = decode_payload(fixture, WireFormat::Legacy).expect("decode lossy fixture");

    assert_eq!(cwd, "C:\\wd");
    assert_eq!(args, args_of(&["\u{FFFD}", "app", "\u{FFFD}"]));
}

#[test]
fn json_round_trip_feeds_paste_of_embedded_pipe_into_launch_context() {
    let argv = args_of(&["tabby-rs", "paste", "A|B"]);
    let cwd = "C:\\work 目錄";
    let bytes = encode_payload(cwd, &argv, WireFormat::Json).expect("json encode");
    let (decoded_argv, decoded_cwd) =
        decode_payload(&bytes, WireFormat::Json).expect("json decode");

    let context = crate::launch::parse_launch_context(&decoded_argv, decoded_cwd, true);

    assert_eq!(context.parse_error, None);
    assert!(context.second_instance);
    assert_eq!(context.cwd, cwd);
    assert_eq!(context.request.argv.text.as_deref(), Some("A|B"));
}

#[test]
fn json_round_trip_feeds_run_command_with_pipe_unicode_and_empty_argument() {
    let argv = args_of(&["tabby-rs", "run", "command.exe", "A|B", "中文", ""]);
    let cwd = "C:\\work 目錄";
    let bytes = encode_payload(cwd, &argv, WireFormat::Json).expect("json encode");
    let (decoded_argv, decoded_cwd) =
        decode_payload(&bytes, WireFormat::Json).expect("json decode");

    let context = crate::launch::parse_launch_context(&decoded_argv, decoded_cwd, true);

    assert_eq!(context.parse_error, None);
    assert!(context.second_instance);
    assert_eq!(context.cwd, cwd);
    assert_eq!(
        context.request.argv.command,
        args_of(&["command.exe", "A|B", "中文", ""])
    );
    assert_eq!(context.request.command, context.request.argv.command);
}
