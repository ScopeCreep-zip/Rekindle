//! The `.tgz` theme reader. Synthetic archives only: every tar header and
//! gzip member is written here (no third-party theme is committed).

use super::tests::{imported, theme, ICONS, SOUNDS};
use super::*;

#[derive(Clone)]
struct Entry {
    name: Vec<u8>,
    flag: u8,
    data: Vec<u8>,
    gnu: bool,
    /// Header bytes overwritten after the fields, before the checksum.
    patch: Vec<(usize, Vec<u8>)>,
    /// The checksum written off by one.
    bad_sum: bool,
}

fn entry(name: &str, flag: u8, data: &[u8]) -> Entry {
    Entry {
        name: name.as_bytes().to_vec(),
        flag,
        data: data.to_vec(),
        gnu: false,
        patch: Vec::new(),
        bad_sum: false,
    }
}

fn header(e: &Entry) -> [u8; 512] {
    let mut h = [0u8; 512];
    h[..e.name.len()].copy_from_slice(&e.name);
    h[100..108].copy_from_slice(b"0000644\0");
    h[108..116].copy_from_slice(b"0000000\0");
    h[116..124].copy_from_slice(b"0000000\0");
    let size = if e.flag == b'5' { 0 } else { e.data.len() };
    h[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
    h[136..148].copy_from_slice(b"00000000000\0");
    h[156] = e.flag;
    if e.gnu {
        h[257..265].copy_from_slice(b"ustar  \0");
    } else {
        h[257..265].copy_from_slice(b"ustar\x0000");
    }
    for (at, bytes) in &e.patch {
        h[*at..*at + bytes.len()].copy_from_slice(bytes);
    }
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|b| u32::from(*b)).sum::<u32>() + u32::from(e.bad_sum);
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    h
}

/// The archive body: headers and padded data, no end blocks.
fn body(entries: &[Entry]) -> Vec<u8> {
    let mut out = Vec::new();
    for e in entries {
        out.extend_from_slice(&header(e));
        if e.flag != b'5' {
            out.extend_from_slice(&e.data);
            out.resize(out.len().div_ceil(512) * 512, 0);
        }
    }
    out
}

/// A whole tar stream: the body, two zero blocks, record padding.
fn tar(entries: &[Entry]) -> Vec<u8> {
    let mut out = body(entries);
    out.resize(out.len() + 1024, 0);
    out.resize(out.len().div_ceil(10240) * 10240, 0);
    out
}

fn gz_with(header: &[u8], data: &[u8]) -> Vec<u8> {
    let mut out = header.to_vec();
    out.extend(miniz_oxide::deflate::compress_to_vec(data, 6));
    out.extend(crc32fast::hash(data).to_le_bytes());
    out.extend(u32::try_from(data.len()).unwrap().to_le_bytes());
    out
}

fn gz(data: &[u8]) -> Vec<u8> {
    gz_with(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff], data)
}

/// The synthetic theme as archive entries under `Synth/`, directories first.
fn theme_entries(gnu: bool) -> Vec<Entry> {
    let mut out = Vec::new();
    for d in [
        "Synth/",
        "Synth/purple/",
        "Synth/purple/blist/",
        "Synth/purple/sound/",
        "Synth/purple/status-icon/",
        "Synth/purple/status-icon/16/",
    ] {
        out.push(entry(d, b'5', b""));
    }
    for (path, bytes) in theme(ICONS, SOUNDS) {
        out.push(entry(&path, b'0', &bytes));
    }
    for e in &mut out {
        e.gnu = gnu;
    }
    out
}

fn with(extra: Entry) -> Vec<u8> {
    let mut all = theme_entries(false);
    all.push(extra);
    tar(&all)
}

#[track_caller]
fn refused_with<T: std::fmt::Debug>(r: Result<T, ImportError>, keyword: &str) {
    match r {
        Err(ImportError::Archive { ref detail, .. }) if detail.contains(keyword) => {}
        other => panic!("expected a refusal naming {keyword:?}, got {other:?}"),
    }
}

fn read_tar(stream: &[u8]) -> Result<Tree, ImportError> {
    tar::read(stream, MAX_UNCOMPRESSED_BYTES)
}

#[test]
fn a_theme_imports_identically_from_its_directory_and_its_tgz() {
    let dir: Tree = theme(ICONS, SOUNDS)
        .into_iter()
        .map(|(k, v)| (k.strip_prefix("Synth/").unwrap().to_owned(), v))
        .collect();
    let from_dir = imported(import(&dir, "synth").unwrap());
    for gnu in [false, true] {
        let tgz = gz(&tar(&theme_entries(gnu)));
        assert_eq!(unpack(&tgz).unwrap(), dir);
        let from_tgz = imported(import_archive(&tgz, "synth").unwrap());
        assert_eq!(from_tgz.canonical.bytes, from_dir.canonical.bytes);
        assert_eq!(from_tgz.notice, from_dir.notice);
    }
    // A ustar `prefix` joins the name.
    let mut e = entry("x.txt", b'0', b"x");
    e.patch.push((345, b"Synth/purple".to_vec()));
    assert!(read_tar(&with(e)).unwrap().contains_key("purple/x.txt"));
    // Every optional gzip header field, FHCRC verified.
    let mut h = vec![0x1f, 0x8b, 8, 0x1e, 0, 0, 0, 0, 0, 3, 2, 0, b'a', b'b'];
    h.extend_from_slice(b"theme.tar\0comment\0");
    let crc = (crc32fast::hash(&h) & 0xffff) as u16;
    h.extend(crc.to_le_bytes());
    assert_eq!(
        unpack(&gz_with(&h, &tar(&theme_entries(false)))).unwrap(),
        dir
    );
    let last = h.len() - 1;
    h[last] ^= 1;
    refused_with(unpack(&gz_with(&h, b"x")), "header CRC-16");
}

#[test]
fn every_entry_but_a_file_or_directory_is_refused() {
    for (flag, why) in [
        (b'1', "hard link"),
        (b'2', "symbolic link"),
        (b'3', "character device"),
        (b'4', "block device"),
        (b'6', "FIFO"),
        (b'7', "contiguous"),
        (b'x', "pax extended"),
        (b'g', "pax extended"),
        (b'L', "GNU long name"),
        (b'K', "GNU long name"),
        (b'S', "GNU sparse"),
        (b'V', "unknown entry type"),
    ] {
        refused_with(read_tar(&with(entry("Synth/x", flag, b""))), why);
    }
    let mut e = entry("Synth/x", b'0', b"");
    e.patch.push((157, b"/etc/passwd".to_vec()));
    refused_with(read_tar(&with(e)), "link name");
}

#[test]
fn unsafe_and_duplicate_paths_are_refused() {
    for name in [
        "/Synth/x",
        "Synth/../x",
        "Synth/./x",
        "Synth//x",
        "Synth\\x",
        "C:/x",
    ] {
        refused_with(
            read_tar(&with(entry(name, b'0', b"x"))),
            "plain relative path",
        );
    }
    let mut e = entry("", b'0', b"x");
    e.name = b"Synth/a\0b".to_vec();
    refused_with(read_tar(&with(e)), "NUL inside the name");
    let mut e = entry("", b'0', b"x");
    e.name = b"Synth/\xff".to_vec();
    refused_with(read_tar(&with(e)), "non-UTF-8");
    let dup = entry("Synth/purple/sound/login.wav", b'0', b"x");
    refused_with(read_tar(&with(dup)), "second entry");
    let dup = entry("Synth/purple/", b'5', b"");
    refused_with(read_tar(&with(dup)), "second entry");
    let under = entry("Synth/purple/sound/login.wav/x", b'0', b"x");
    refused_with(read_tar(&with(under)), "is a file");
}

#[test]
fn an_archive_without_one_top_level_directory_is_refused() {
    refused_with(
        read_tar(&with(entry("Other/x", b'0', b"x"))),
        "more than one top-level",
    );
    refused_with(
        read_tar(&tar(&[entry("theme.xml", b'0', b"x")])),
        "a file at the top level",
    );
    refused_with(read_tar(&tar(&[entry("Synth/", b'5', b"")])), "no files");
}

#[test]
fn a_bad_header_or_ending_is_refused() {
    let mut e = entry("Synth/x", b'0', b"x");
    e.bad_sum = true;
    refused_with(read_tar(&with(e)), "checksum mismatch");
    let mut e = entry("Synth/x", b'0', b"x");
    e.patch.push((257, vec![0; 8]));
    refused_with(read_tar(&with(e)), "not a ustar header");
    let mut e = entry("Synth/x", b'0', b"x");
    e.patch
        .push((124, vec![0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]));
    refused_with(read_tar(&with(e)), "base-256");
    let mut d = entry("Synth/d/", b'5', b"");
    d.patch.push((124, b"00000000001\0".to_vec()));
    refused_with(read_tar(&with(d)), "directory with a size");
    let all = theme_entries(false);
    refused_with(read_tar(&body(&all)), "truncated");
    let mut one = body(&all);
    one.resize(one.len() + 512, 0);
    refused_with(read_tar(&one), "two zero blocks");
    let mut trailing = tar(&all);
    let last = trailing.len() - 1;
    trailing[last] = 1;
    refused_with(read_tar(&trailing), "zero padding");
}

#[test]
fn the_entry_file_and_total_caps_refuse_before_copying() {
    let many: Vec<Entry> = (0..=MAX_ENTRIES)
        .map(|i| entry(&format!("Synth/{i}"), b'0', b""))
        .collect();
    refused_with(read_tar(&tar(&many)), "entries past");
    // A declared size past the file cap, with no data behind it.
    let mut e = entry("Synth/big", b'0', b"");
    let size = format!("{:011o}\0", tar::MAX_THEME_FILE_BYTES + 1);
    e.patch.push((124, size.into_bytes()));
    refused_with(read_tar(&with(e)), "is past");
    let two = [
        entry("Synth/a", b'0', &[1; 600]),
        entry("Synth/b", b'0', &[1; 600]),
    ];
    refused_with(tar::read(&tar(&two), 1000), "1200 bytes is past 1000");
    assert_eq!(tar::read(&tar(&two), 1200).unwrap().len(), 2);
}

#[test]
fn a_gzip_bomb_and_a_bad_member_are_refused() {
    // An honest bomb: 1 MiB of zeros in about a kilobyte, past the cap.
    let bomb = gz(&vec![0; 1 << 20]);
    assert!(bomb.len() < 8 * 1024);
    refused_with(gzip::gunzip(&bomb, 64 * 1024), "decompressed is past");
    // The real cap, an ISIZE past it: refused before any allocation.
    let mut past = gz(b"x");
    let n = past.len();
    let over = u32::try_from(MAX_UNCOMPRESSED_BYTES + 1).unwrap();
    past[n - 4..].copy_from_slice(&over.to_le_bytes());
    refused_with(unpack(&past), "decompressed is past");
    // A lying ISIZE: inflation stops at it.
    let mut lying = bomb.clone();
    let n = lying.len();
    lying[n - 4..].copy_from_slice(&1024u32.to_le_bytes());
    refused_with(unpack(&lying), "inflates past ISIZE");
    let mut short = gz(b"abc");
    let n = short.len();
    short[n - 4..].copy_from_slice(&4u32.to_le_bytes());
    refused_with(unpack(&short), "ISIZE says");
    let mut crc = gz(b"abc");
    crc[n - 8] ^= 1;
    refused_with(unpack(&crc), "CRC-32 mismatch");
    let mut two = gz(b"abc");
    two.extend(gz(b"def"));
    refused_with(unpack(&two), "after the deflate stream");
    refused_with(unpack(b"PK\x03\x04 not a gzip"), "not a gzip");
    let mut reserved = gz(b"abc");
    reserved[3] = 0x20;
    refused_with(unpack(&reserved), "reserved flag");
    refused_with(
        unpack(&vec![0; usize::try_from(MAX_ARCHIVE_BYTES).unwrap() + 1]),
        "is past",
    );
}
