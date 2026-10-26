//! Delta downloads with zsync. A release that publishes `<asset>.zsync`
//! next to an AppImage lists a rolling checksum and an MD4 checksum for
//! each block of it. The installed version is scanned for blocks the new
//! one shares, those are copied, and only the rest is fetched with HTTP
//! range requests. The result is checked against the SHA-1 in the control
//! file, and then like any download.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use md4::Md4;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use ureq::ResponseExt;

use crate::error::{Error, Result};
use crate::http;
use crate::model::Asset;

/// A parsed `.zsync` control file.
#[derive(Debug)]
pub struct Control {
    /// Size of the target file.
    pub length: u64,
    pub blocksize: usize,
    /// Consecutive blocks that must match together (1 or 2), which lets
    /// the checksums be shorter.
    seq_matches: usize,
    rsum_bytes: usize,
    checksum_bytes: usize,
    /// SHA-1 of the target file, lowercase hex.
    sha1: String,
    blocks: Vec<BlockSum>,
}

#[derive(Debug)]
struct BlockSum {
    /// The rolling checksum's trailing `rsum_bytes` bytes, as a number.
    rsum: u32,
    /// The MD4 checksum's leading `checksum_bytes` bytes.
    checksum: [u8; 16],
}

/// How a delta download went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    /// Bytes taken from the installed version.
    pub reused: u64,
    /// Size of the new version.
    pub length: u64,
}

/// Below this share of the file in common, a delta saves too little for
/// the many requests it costs; the caller downloads the file whole.
const MIN_REUSED: f64 = 0.05;
/// Missing ranges closer than this are fetched in one request, re-fetching
/// the bytes between them.
const MERGE_GAP: u64 = 64 * 1024;
/// Most range requests one download makes; gaps are merged until it fits.
const MAX_REQUESTS: usize = 1000;
/// How much of the installed file is read at a time.
const CHUNK: usize = 4 * 1024 * 1024;

pub fn parse(data: &[u8]) -> Result<Control> {
    let bad = |what: &str| Error::Delta(format!("bad control file: {what}"));
    let end = data
        .windows(2)
        .position(|w| w == b"\n\n")
        .ok_or_else(|| bad("no end of header"))?;
    let header = String::from_utf8_lossy(&data[..end]);
    let field = |key: &str| {
        header.lines().find_map(|line| {
            let (k, v) = line.split_once(':')?;
            (k.trim() == key).then(|| v.trim().to_string())
        })
    };
    let number = |key: &str| -> Result<u64> {
        field(key)
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| bad(&format!("no {key}")))
    };
    let length = number("Length")?;
    let blocksize = number("Blocksize")? as usize;
    if blocksize == 0 || blocksize > 1 << 24 {
        return Err(bad("block size"));
    }
    let lengths: Vec<usize> = field("Hash-Lengths")
        .ok_or_else(|| bad("no Hash-Lengths"))?
        .split(',')
        .map(|n| n.trim().parse().map_err(|_| bad("Hash-Lengths")))
        .collect::<Result<_>>()?;
    let [seq_matches, rsum_bytes, checksum_bytes] = lengths[..] else {
        return Err(bad("Hash-Lengths"));
    };
    if !(1..=2).contains(&seq_matches) || !(1..=4).contains(&rsum_bytes) || !(3..=16).contains(&checksum_bytes) {
        return Err(bad("Hash-Lengths"));
    }
    let sha1 = field("SHA-1")
        .filter(|h| h.len() == 40 && h.chars().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(|| bad("no SHA-1"))?
        .to_ascii_lowercase();

    let count = length.div_ceil(blocksize as u64) as usize;
    let entry = rsum_bytes + checksum_bytes;
    let sums = &data[end + 2..];
    if sums.len() < count * entry {
        return Err(bad("too few block checksums"));
    }
    let blocks = sums
        .chunks_exact(entry)
        .take(count)
        .map(|b| {
            let mut rsum = [0u8; 4];
            rsum[4 - rsum_bytes..].copy_from_slice(&b[..rsum_bytes]);
            let mut checksum = [0u8; 16];
            checksum[..checksum_bytes].copy_from_slice(&b[rsum_bytes..]);
            BlockSum {
                rsum: u32::from_be_bytes(rsum),
                checksum,
            }
        })
        .collect();
    Ok(Control {
        length,
        blocksize,
        seq_matches,
        rsum_bytes,
        checksum_bytes,
        sha1,
        blocks,
    })
}

/// zsync's rolling checksum of a block: the byte sum and the sum weighted
/// by distance from the end, each modulo 2^16.
fn rsum(block: &[u8]) -> (u16, u16) {
    let len = block.len();
    block.iter().enumerate().fold((0u16, 0u16), |(a, b), (i, &c)| {
        (
            a.wrapping_add(c.into()),
            b.wrapping_add(((len - i) as u16).wrapping_mul(c.into())),
        )
    })
}

fn md4(block: &[u8]) -> [u8; 16] {
    Md4::digest(block).into()
}

impl Control {
    fn key(&self, (a, b): (u16, u16)) -> u32 {
        let full = (u32::from(a) << 16) | u32::from(b);
        full & (u32::MAX >> (8 * (4 - self.rsum_bytes)))
    }

    fn block_matches(&self, i: usize, window: &[u8]) -> bool {
        let n = self.checksum_bytes;
        self.key(rsum(window)) == self.blocks[i].rsum && md4(window)[..n] == self.blocks[i].checksum[..n]
    }

    /// Where in `seed` each block of the target is found, if anywhere.
    /// The seed is read as if followed by zeros, which is how the target's
    /// last, short block was checksummed.
    fn locate(&self, seed: &mut impl Read) -> io::Result<Vec<Option<u64>>> {
        let bs = self.blocksize;
        let n = self.blocks.len();
        let mut index: HashMap<u32, Vec<usize>> = HashMap::new();
        for (i, block) in self.blocks.iter().enumerate() {
            index.entry(block.rsum).or_default().push(i);
        }
        let mut found = vec![None; n];
        let window_needed = bs * self.seq_matches;
        let (mut buf, mut base, mut pos, mut eof) = (Vec::with_capacity(CHUNK + 2 * bs), 0u64, 0usize, false);
        let mut sum: Option<(u16, u16)> = None;
        loop {
            // Room for the window, the one after it, and a byte to roll in.
            if buf.len() < pos + window_needed + 1 && !eof {
                buf.drain(..pos);
                base += pos as u64;
                pos = 0;
                let start = buf.len();
                buf.resize(start + CHUNK, 0);
                let read = read_full(seed, &mut buf[start..])?;
                buf.truncate(start + read);
                if read < CHUNK {
                    eof = true;
                    buf.resize(buf.len() + window_needed, 0);
                }
                continue;
            }
            if buf.len() < pos + bs {
                break;
            }
            let (a, b) = *sum.get_or_insert_with(|| rsum(&buf[pos..pos + bs]));
            let mut matched = false;
            if let Some(candidates) = index.get(&self.key((a, b))) {
                let n_bytes = self.checksum_bytes;
                let mut checksum = None;
                for &i in candidates {
                    if found[i].is_some() {
                        continue;
                    }
                    let window = &buf[pos..pos + bs];
                    let checksum = checksum.get_or_insert_with(|| md4(window));
                    if checksum[..n_bytes] != self.blocks[i].checksum[..n_bytes] {
                        continue;
                    }
                    if self.seq_matches > 1 && i + 1 < n {
                        let next = buf.get(pos + bs..pos + 2 * bs);
                        if !next.is_some_and(|next| self.block_matches(i + 1, next)) {
                            continue;
                        }
                    }
                    found[i] = Some(base + pos as u64);
                    matched = true;
                }
            }
            if matched {
                pos += bs;
                sum = None;
                continue;
            }
            if pos + bs >= buf.len() {
                break;
            }
            let (old, new) = (u16::from(buf[pos]), u16::from(buf[pos + bs]));
            let a = a.wrapping_add(new).wrapping_sub(old);
            let b = b.wrapping_add(a).wrapping_sub(old.wrapping_mul(bs as u16));
            sum = Some((a, b));
            pos += 1;
        }
        Ok(found)
    }

    /// Byte ranges of the target missing from `found`, with near ones merged.
    fn missing_ranges(&self, found: &[Option<u64>]) -> Vec<(u64, u64)> {
        let bs = self.blocksize as u64;
        let mut ranges: Vec<(u64, u64)> = Vec::new();
        for (i, _) in found.iter().enumerate().filter(|(_, f)| f.is_none()) {
            let (start, end) = (i as u64 * bs, ((i as u64 + 1) * bs).min(self.length));
            match ranges.last_mut() {
                Some(last) if last.1 == start => last.1 = end,
                _ => ranges.push((start, end)),
            }
        }
        let mut gap = MERGE_GAP;
        loop {
            let mut merged: Vec<(u64, u64)> = Vec::new();
            for &(start, end) in &ranges {
                match merged.last_mut() {
                    Some(last) if start - last.1 <= gap => last.1 = end,
                    _ => merged.push((start, end)),
                }
            }
            if merged.len() <= MAX_REQUESTS {
                return merged;
            }
            ranges = merged;
            gap *= 2;
        }
    }
}

fn read_full(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut done = 0;
    while done < buf.len() {
        match reader.read(&mut buf[done..]) {
            Ok(0) => break,
            Ok(n) => done += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(done)
}

/// Downloads `asset` into `dir` using `seed`, an older version of it, and
/// the zsync control file at `control_url`. Returns the file, its SHA-256
/// and how much was reused, as `verify::download` does. Any error but
/// `Cancelled` means the caller should download the file whole.
pub fn download(
    asset: &Asset,
    control_url: &str,
    seed: &Path,
    dir: &Path,
    progress: &mut dyn FnMut(u64, Option<u64>) -> ControlFlow<()>,
) -> Result<(PathBuf, String, Stats)> {
    let control = parse(&http::get_bytes(control_url)?)?;
    if asset.size > 0 && asset.size != control.length {
        return Err(Error::Delta(format!(
            "control file is for a {} byte file, the asset has {}",
            control.length, asset.size
        )));
    }
    let found = control.locate(&mut io::BufReader::new(File::open(seed)?))?;
    let reused = found
        .iter()
        .enumerate()
        .filter(|(_, f)| f.is_some())
        .map(|(i, _)| block_len(&control, i))
        .sum::<u64>();
    if (reused as f64) < control.length as f64 * MIN_REUSED {
        return Err(Error::Delta("too little in common with the installed version".into()));
    }

    fs::create_dir_all(dir)?;
    let file_name = Path::new(&asset.name)
        .file_name()
        .ok_or_else(|| Error::Delta(format!("bad asset name {}", asset.name)))?;
    let path = dir.join(file_name);
    let mut part = path.clone().into_os_string();
    part.push(".part");
    let part = PathBuf::from(part);
    match rebuild(&control, &found, seed, &asset.url, &part, progress) {
        Ok(hash) => {
            fs::rename(&part, &path)?;
            Ok((
                path,
                hash,
                Stats {
                    reused,
                    length: control.length,
                },
            ))
        }
        Err(e) => {
            fs::remove_file(&part).ok();
            Err(e)
        }
    }
}

fn block_len(control: &Control, i: usize) -> u64 {
    let bs = control.blocksize as u64;
    (control.length - i as u64 * bs).min(bs)
}

/// Writes the target to `out`: found blocks from `seed`, the rest from
/// `url`. Returns its SHA-256 once its SHA-1 matches the control file's.
fn rebuild(
    control: &Control,
    found: &[Option<u64>],
    seed: &Path,
    url: &str,
    out: &Path,
    progress: &mut dyn FnMut(u64, Option<u64>) -> ControlFlow<()>,
) -> Result<String> {
    let mut file = File::create(out)?;
    file.set_len(control.length)?;
    let mut seed = File::open(seed)?;
    let mut block = vec![0u8; control.blocksize];
    for (i, offset) in found.iter().enumerate() {
        let Some(offset) = offset else { continue };
        let len = block_len(control, i) as usize;
        seed.seek(SeekFrom::Start(*offset))?;
        // Past the end of the seed are the zeros it was matched with.
        let read = read_full(&mut seed, &mut block[..len])?;
        block[read..len].fill(0);
        file.seek(SeekFrom::Start(i as u64 * control.blocksize as u64))?;
        file.write_all(&block[..len])?;
    }

    let ranges = control.missing_ranges(found);
    let total: u64 = ranges.iter().map(|(start, end)| end - start).sum();
    let mut done = 0;
    if progress(0, Some(total)).is_break() {
        return Err(Error::Cancelled);
    }
    // Redirects (GitHub's lead to a signed address) are followed once and
    // the address they end at used for the other ranges; if it stops
    // working, e.g. its signature expired, it is looked up again.
    let mut target = url.to_string();
    let mut buf = vec![0u8; 64 * 1024];
    for &(start, end) in &ranges {
        let mut response = match fetch_range(&target, start, end) {
            Ok(response) => response,
            Err(_) if target != url => {
                target = url.to_string();
                fetch_range(&target, start, end)?
            }
            Err(e) => return Err(e),
        };
        target = response.get_uri().to_string();
        file.seek(SeekFrom::Start(start))?;
        let mut reader = response.body_mut().as_reader().take(end - start);
        let mut got = 0;
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            };
            file.write_all(&buf[..n])?;
            got += n as u64;
            done += n as u64;
            if progress(done, Some(total)).is_break() {
                return Err(Error::Cancelled);
            }
        }
        if got != end - start {
            return Err(Error::Delta(format!("range {start}-{end} ended after {got} bytes")));
        }
    }
    file.sync_all()?;
    drop(file);

    let (mut sha1, mut sha256) = (Sha1::new(), Sha256::new());
    let mut file = File::open(out)?;
    loop {
        let n = read_full(&mut file, &mut buf)?;
        if n == 0 {
            break;
        }
        sha1.update(&buf[..n]);
        sha256.update(&buf[..n]);
    }
    if hex::encode(sha1.finalize()) != control.sha1 {
        return Err(Error::Delta(
            "rebuilt file does not match the control file's SHA-1".into(),
        ));
    }
    Ok(hex::encode(sha256.finalize()))
}

/// Requests bytes `start..end` of `url`, insisting on a partial answer:
/// a server that sends the whole file instead would cost more than a
/// plain download.
fn fetch_range(url: &str, start: u64, end: u64) -> Result<ureq::http::Response<ureq::Body>> {
    let response = http::check(
        http::get(url)
            .header("Range", &format!("bytes={start}-{}", end - 1))
            .call()?,
    )?;
    let content_range = response
        .headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if response.status() != 206 || !content_range.starts_with(&format!("bytes {start}-")) {
        return Err(Error::Delta(format!("{url} does not serve byte ranges")));
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    /// A control file for `data`, written the way zsyncmake writes one.
    fn make_control(data: &[u8], blocksize: usize, seq: usize, rsum_bytes: usize, checksum_bytes: usize) -> Vec<u8> {
        let mut out = format!(
            "zsync: 0.6.2\nFilename: tool.AppImage\nBlocksize: {blocksize}\nLength: {}\n\
             Hash-Lengths: {seq},{rsum_bytes},{checksum_bytes}\nURL: tool.AppImage\nSHA-1: {}\n\n",
            data.len(),
            hex::encode(Sha1::digest(data))
        )
        .into_bytes();
        for chunk in data.chunks(blocksize) {
            let mut block = chunk.to_vec();
            block.resize(blocksize, 0);
            let (a, b) = rsum(&block);
            let mut sums = [a.to_be_bytes(), b.to_be_bytes()].concat();
            sums.extend_from_slice(&md4(&block));
            out.extend_from_slice(&sums[4 - rsum_bytes..4 + checksum_bytes]);
        }
        out
    }

    /// Bytes that do not repeat, so blocks only match where they came from.
    fn noise(len: usize, seed: u64) -> Vec<u8> {
        let mut x = seed;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect()
    }

    #[test]
    fn rolling_checksum_rolls() {
        let data = noise(300, 1);
        let bs = 64;
        let (mut a, mut b) = rsum(&data[..bs]);
        for pos in 0..data.len() - bs {
            let (old, new) = (u16::from(data[pos]), u16::from(data[pos + bs]));
            a = a.wrapping_add(new).wrapping_sub(old);
            b = b.wrapping_add(a).wrapping_sub(old.wrapping_mul(bs as u16));
            assert_eq!((a, b), rsum(&data[pos + 1..pos + 1 + bs]));
        }
    }

    #[test]
    fn parses_a_control_file() {
        let data = noise(10_000, 2);
        let control = parse(&make_control(&data, 2048, 2, 3, 5)).unwrap();
        assert_eq!(control.length, 10_000);
        assert_eq!(control.blocks.len(), 5);
        assert_eq!(
            (control.seq_matches, control.rsum_bytes, control.checksum_bytes),
            (2, 3, 5)
        );
        assert!(parse(b"zsync: 0.6.2\nBlocksize: 2048\n\n").is_err());
    }

    /// The new version: the old with bytes inserted near the start, a
    /// changed stretch in the middle, and a different end.
    fn versions() -> (Vec<u8>, Vec<u8>) {
        let old = noise(1024 * 1024 + 100, 3);
        let mut new = old.clone();
        new.splice(500..500, noise(37, 4));
        new.splice(30_000..32_000, noise(3_000, 5));
        new.truncate(new.len() - 700);
        new.extend(noise(1_234, 6));
        (old, new)
    }

    #[test]
    fn finds_shifted_blocks_in_the_old_version() {
        let (old, new) = versions();
        for (seq, rsum_bytes, checksum_bytes) in [(1, 4, 16), (2, 2, 3), (2, 3, 4)] {
            let control = parse(&make_control(&new, 1024, seq, rsum_bytes, checksum_bytes)).unwrap();
            let found = control.locate(&mut &old[..]).unwrap();
            for (i, offset) in found.iter().enumerate() {
                if let Some(offset) = offset {
                    let len = block_len(&control, i) as usize;
                    let start = i * 1024;
                    assert_eq!(&old[*offset as usize..*offset as usize + len], &new[start..start + len]);
                }
            }
            let reused = found.iter().filter(|f| f.is_some()).count();
            // All but the blocks around the three changes.
            assert!(reused >= found.len() - 12, "{reused} of {}", found.len());
        }
    }

    #[test]
    fn merges_nearby_missing_ranges() {
        let control = parse(&make_control(&noise(100 * 1024, 7), 1024, 1, 4, 16)).unwrap();
        let mut found: Vec<Option<u64>> = (0..100).map(|i| Some(i * 1024)).collect();
        for i in [3, 4, 10, 99] {
            found[i] = None;
        }
        assert_eq!(
            control.missing_ranges(&found),
            vec![(3 * 1024, 11 * 1024), (99 * 1024, 100 * 1024)]
        );
    }

    /// Serves `body` over HTTP on a local port, answering range requests
    /// with 206, and counts the requests.
    fn serve_ranges(body: Vec<u8>) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::sync::atomic::Ordering;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/tool.AppImage", listener.local_addr().unwrap());
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = count.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut range = None;
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    loop {
                        let mut header = String::new();
                        reader.read_line(&mut header).unwrap();
                        if header.trim().is_empty() {
                            break;
                        }
                        if let Some(value) = header.to_ascii_lowercase().strip_prefix("range: bytes=") {
                            let (s, e) = value.trim().split_once('-').unwrap();
                            range = Some((s.parse::<usize>().unwrap(), e.parse::<usize>().unwrap()));
                        }
                    }
                    counter.fetch_add(1, Ordering::Relaxed);
                    let (s, e) = range.unwrap();
                    let head = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {s}-{e}/{}\r\n\r\n",
                        e + 1 - s,
                        body.len()
                    );
                    stream.write_all(head.as_bytes()).unwrap();
                    stream.write_all(&body[s..=e]).unwrap();
                }
            }
        });
        (url, count)
    }

    #[test]
    fn rebuilds_the_new_version_fetching_only_what_changed() {
        let (old, new) = versions();
        let control = parse(&make_control(&new, 1024, 2, 3, 5)).unwrap();
        let found = control.locate(&mut &old[..]).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let seed = tmp.path().join("old.AppImage");
        fs::write(&seed, &old).unwrap();
        let (url, requests) = serve_ranges(new.clone());
        let out = tmp.path().join("new.AppImage");
        let mut fetched = 0;
        let hash = rebuild(&control, &found, &seed, &url, &out, &mut |done, _| {
            fetched = done;
            ControlFlow::Continue(())
        })
        .unwrap();
        assert_eq!(fs::read(&out).unwrap(), new);
        assert_eq!(hash, hex::encode(Sha256::digest(&new)));
        assert!(fetched < new.len() as u64 / 4, "fetched {fetched}");
        assert!(requests.load(std::sync::atomic::Ordering::Relaxed) <= 4);
    }

    #[test]
    fn a_wrong_rebuild_fails_the_sha1_check() {
        let (old, new) = versions();
        let mut control = parse(&make_control(&new, 1024, 1, 4, 16)).unwrap();
        control.sha1 = "0".repeat(40);
        let found = control.locate(&mut &old[..]).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let seed = tmp.path().join("old.AppImage");
        fs::write(&seed, &old).unwrap();
        let (url, _) = serve_ranges(new);
        let result = rebuild(&control, &found, &seed, &url, &tmp.path().join("out"), &mut |_, _| {
            ControlFlow::Continue(())
        });
        assert!(matches!(result, Err(Error::Delta(_))));
    }
}
