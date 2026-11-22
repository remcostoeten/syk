use std::fs;

use crate::{Fixture, forget};

#[test]
fn large_binary_and_wide_files_are_read_like_small_ones() {
    let fixture = Fixture::new();
    let filler = "plain text line that fills the file\n".repeat(2500);
    let write = |path: &str, bytes: Vec<u8>| {
        fs::write(fixture.root.join(path), bytes).expect("write file");
    };
    fixture.dir("edge");
    write(
        "edge/large.txt",
        format!("{filler}sought at the end\n").into_bytes(),
    );
    write(
        "edge/late.bin",
        format!("{filler}sought before a zero\n\0tail\n").into_bytes(),
    );
    write("edge/early.bin", b"sought\0\n".to_vec());
    write(
        "edge/exact.txt",
        format!("{}\nsought after one buffer\n", "x".repeat(65535)).into_bytes(),
    );
    write("edge/noeol.txt", b"sought without newline".to_vec());
    write("edge/empty.txt", Vec::new());
    let mut wide = vec![0xFF, 0xFE];
    wide.extend(
        "first\nsought in utf16\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes),
    );
    write("edge/wide.txt", wide);
    assert_eq!(
        fixture.lines(&["-c", "sought", "-p"]),
        [
            "edge/exact.txt:2:sought after one buffer",
            "edge/large.txt:2501:sought at the end",
            "edge/noeol.txt:1:sought without newline",
            "edge/wide.txt:2:sought in utf16",
        ]
    );
    assert_eq!(
        fixture.lines(&["-c", "sought", "-N"]),
        [
            "edge/exact.txt:1",
            "edge/large.txt:1",
            "edge/noeol.txt:1",
            "edge/wide.txt:1",
        ]
    );
}

#[test]
fn files_are_found_whether_or_not_their_data_is_in_memory() {
    for count in [40, 400] {
        let fixture = Fixture::new();
        let mut wanted = Vec::new();
        for index in 0..count {
            let path = format!("many/{}/{index}.txt", index % 7);
            fixture.file(&path, &format!("filler\nsought {index}\n"));
            forget(&fixture.root.join(&path));
            wanted.push(format!("{path}:2:sought {index}"));
        }
        wanted.sort();
        let wanted: Vec<&str> = wanted.iter().map(String::as_str).collect();
        for _ in 0..2 {
            let (printed, status) = fixture.run(&["-c", "sought", "-p"]);
            let mut lines: Vec<&str> = printed.lines().collect();
            lines.sort();
            assert_eq!((lines, status), (wanted.clone(), 0));
        }
    }
}

#[test]
fn heavy_files_in_a_small_tree_are_all_searched() {
    let fixture = Fixture::new();
    let text = "sought on every line\n".repeat(20_000);
    let mut wanted = Vec::new();
    for index in 0..8 {
        fixture.file(&format!("heavy/{index}.txt"), &text);
        wanted.push(format!("heavy/{index}.txt:20000"));
    }
    let wanted: Vec<&str> = wanted.iter().map(String::as_str).collect();
    let (printed, status) = fixture.run(&["-c", "sought", "-N"]);
    let mut lines: Vec<&str> = printed.lines().collect();
    lines.sort();
    assert_eq!((lines, status), (wanted, 0));
}
