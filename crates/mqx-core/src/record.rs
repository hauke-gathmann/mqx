use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
    thread::JoinHandle,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::{
    error::{Error, Result},
    message::Inbound,
};

pub const RECORDING_KIND: &str = "mqx-recording";
pub const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;
pub const RECORDER_QUEUE: usize = 8192;
pub const MAX_RECORDING_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingHeader {
    pub kind: String,
    pub v: u32,
    pub started_at: String,
    pub ended_at: String,
    pub profile_id: String,
    pub profile_name: String,
    pub broker: String,
    pub messages: u64,
    pub topics: u64,
    pub app_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordEvent {
    pub t_ms: u64,
    pub topic: String,
    pub qos: u8,
    pub retain: bool,
    #[serde(default)]
    pub dup: bool,
    pub payload: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordingLine {
    Header(RecordingHeader),
    Event(RecordEvent),
}

impl RecordingHeader {
    pub fn to_jsonl(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|source| Error::Recording(source.to_string()))
    }

    pub fn from_jsonl(line: &str) -> Result<Self> {
        let line = line.trim();
        if line.is_empty() {
            return Err(Error::Recording("empty recording header".into()));
        }
        serde_json::from_str(line).map_err(|source| Error::Recording(source.to_string()))
    }
}

impl RecordEvent {
    pub fn from_inbound(inbound: &Inbound) -> Self {
        Self {
            t_ms: unix_ms(inbound.timestamp),
            topic: inbound.topic.clone(),
            qos: inbound.qos.as_u8(),
            retain: inbound.retain,
            dup: inbound.dup,
            payload: BASE64.encode(inbound.payload.as_ref()),
        }
    }

    pub fn payload_bytes(&self) -> Result<Vec<u8>> {
        BASE64
            .decode(self.payload.as_bytes())
            .map_err(|_| Error::Recording("recording payload is not valid base64".into()))
    }

    pub fn to_jsonl(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|source| Error::Recording(source.to_string()))
    }

    pub fn from_jsonl(line: &str) -> Result<Self> {
        let line = line.trim();
        if line.is_empty() {
            return Err(Error::Recording("empty recording line".into()));
        }
        serde_json::from_str(line).map_err(|source| Error::Recording(source.to_string()))
    }
}

/// Header objects have top-level `kind`; mqtt-trace events do not.
pub fn sniff_line(line: &str) -> Result<RecordingLine> {
    let line = line.trim();
    if line.is_empty() {
        return Err(Error::Recording("empty recording line".into()));
    }
    if line_has_kind(line) {
        RecordingHeader::from_jsonl(line).map(RecordingLine::Header)
    } else {
        RecordEvent::from_jsonl(line).map(RecordingLine::Event)
    }
}

pub fn load_recording(
    path: impl AsRef<Path>,
) -> Result<(Option<RecordingHeader>, Vec<RecordEvent>)> {
    let path = path.as_ref();
    let file = std::fs::File::open(path)?;
    load_recording_from(BufReader::new(file)).map_err(|err| err.with_recording_path(path))
}

pub fn check_recording_size(len: u64) -> Result<()> {
    if len > MAX_RECORDING_BYTES {
        Err(Error::Recording("recording exceeds 4 GiB".into()))
    } else {
        Ok(())
    }
}

/// Load events for V2 replay. Refuses files over 4 GiB and headers with `v` > 1.
pub fn load_replay_events(path: impl AsRef<Path>) -> Result<Vec<RecordEvent>> {
    let path = path.as_ref();
    let meta = fs::metadata(path)?;
    check_recording_size(meta.len()).map_err(|err| err.with_recording_path(path))?;
    let (_header, events) = load_recording(path)?;
    if events.is_empty() {
        return Err(
            Error::Recording("recording contains no events".into()).with_recording_path(path)
        );
    }
    Ok(events)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingInfo {
    pub path: String,
    pub name: String,
    pub file_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_t_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_t_ms: Option<u64>,
    pub messages: u64,
    pub topics: u64,
    pub bytes: u64,
    pub mtime_ms: u64,
}

#[derive(Default)]
pub struct RecordingScanCache {
    entries: HashMap<PathBuf, CachedRecording>,
}

struct CachedRecording {
    mtime: SystemTime,
    len: u64,
    info: RecordingInfo,
}

impl RecordingScanCache {
    pub fn get_or_inspect(&mut self, path: &Path, meta: &fs::Metadata) -> Result<RecordingInfo> {
        let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let len = meta.len();
        if let Some(cached) = self.entries.get(path)
            && cached.mtime == mtime
            && cached.len == len
        {
            return Ok(cached.info.clone());
        }
        let info = inspect_recording(path, meta)?;
        self.entries.insert(
            path.to_path_buf(),
            CachedRecording {
                mtime,
                len,
                info: info.clone(),
            },
        );
        Ok(info)
    }
}

/// `*.jsonl` in `dir`, skipping dotfiles. Headerless files are scanned once per mtime.
pub fn list_recordings(dir: &Path, cache: &mut RecordingScanCache) -> Result<Vec<RecordingInfo>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(meta) if meta.is_file() => meta,
            _ => continue,
        };
        seen.insert(path.clone());
        match cache.get_or_inspect(&path, &meta) {
            Ok(info) => out.push(info),
            Err(_) => continue,
        }
    }
    cache.entries.retain(|path, _| seen.contains(path));
    out.sort_by(|a, b| {
        b.mtime_ms
            .cmp(&a.mtime_ms)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

pub fn inspect_recording(path: &Path, meta: &fs::Metadata) -> Result<RecordingInfo> {
    let file = fs::File::open(path)?;
    inspect_recording_from(BufReader::new(file), path, meta)
        .map_err(|err| err.with_recording_path(path))
}

fn inspect_recording_from(
    mut reader: impl BufRead,
    path: &Path,
    meta: &fs::Metadata,
) -> Result<RecordingInfo> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = path
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| file_name.clone());
    let bytes = meta.len();
    let mtime_ms = meta
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let mut header = None;
    let mut first_t_ms = None;
    let mut last_t_ms = None;
    let mut messages = 0u64;
    let mut topics = HashSet::new();
    let mut first_nonempty = true;
    let mut line_no = 0usize;
    while let Some(line) = read_line_capped(&mut reader, MAX_LINE_BYTES)
        .map_err(|err| err.with_recording_line(line_no + 1))?
    {
        line_no += 1;
        if line.trim().is_empty() {
            continue;
        }
        if first_nonempty {
            first_nonempty = false;
            match sniff_line(&line).map_err(|err| err.with_recording_line(line_no))? {
                RecordingLine::Header(parsed) => {
                    header = Some(parsed);
                    break;
                }
                RecordingLine::Event(event) => {
                    first_t_ms = Some(event.t_ms);
                    last_t_ms = Some(event.t_ms);
                    topics.insert(event.topic);
                    messages = 1;
                }
            }
            continue;
        }
        let event =
            RecordEvent::from_jsonl(&line).map_err(|err| err.with_recording_line(line_no))?;
        if first_t_ms.is_none() {
            first_t_ms = Some(event.t_ms);
        }
        last_t_ms = Some(event.t_ms);
        topics.insert(event.topic);
        messages = messages.saturating_add(1);
    }

    if let Some(header) = header {
        return Ok(RecordingInfo {
            path: path.display().to_string(),
            name,
            file_name,
            started_at: Some(header.started_at),
            first_t_ms: None,
            last_t_ms: None,
            messages: header.messages,
            topics: header.topics,
            bytes,
            mtime_ms,
        });
    }

    Ok(RecordingInfo {
        path: path.display().to_string(),
        name,
        file_name,
        started_at: None,
        first_t_ms,
        last_t_ms,
        messages,
        topics: topics.len() as u64,
        bytes,
        mtime_ms,
    })
}

pub fn load_recording_from(
    mut reader: impl BufRead,
) -> Result<(Option<RecordingHeader>, Vec<RecordEvent>)> {
    let mut header = None;
    let mut events = Vec::new();
    let mut first_nonempty = true;
    let mut line_no = 0usize;
    while let Some(line) = read_line_capped(&mut reader, MAX_LINE_BYTES)
        .map_err(|err| err.with_recording_line(line_no + 1))?
    {
        line_no += 1;
        if line.trim().is_empty() {
            continue;
        }
        if first_nonempty {
            first_nonempty = false;
            match sniff_line(&line).map_err(|err| err.with_recording_line(line_no))? {
                RecordingLine::Header(parsed) => {
                    if parsed.v > 1 {
                        return Err(
                            Error::Recording("upgrade mqx".into()).with_recording_line(line_no)
                        );
                    }
                    header = Some(parsed);
                }
                RecordingLine::Event(event) => events.push(event),
            }
        } else {
            events.push(
                RecordEvent::from_jsonl(&line).map_err(|err| err.with_recording_line(line_no))?,
            );
        }
    }
    Ok((header, events))
}

pub fn wait_duration(t0_ms: u64, t_ms: u64, speed: f64, elapsed: Duration) -> Duration {
    if !speed.is_finite() || speed <= 0.0 {
        return Duration::ZERO;
    }
    let recorded = Duration::from_millis(t_ms.saturating_sub(t0_ms));
    let scaled = recorded.div_f64(speed);
    scaled.saturating_sub(elapsed)
}

fn line_has_kind(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line.trim())
        .ok()
        .is_some_and(|value| value.get("kind").is_some())
}

fn read_line_capped(reader: &mut impl BufRead, max: usize) -> Result<Option<String>> {
    let mut buf = Vec::new();
    // +2 so a max-length line ending in `\r\n` is not rejected.
    let n = reader
        .by_ref()
        .take(max as u64 + 2)
        .read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(None);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
    }
    if buf.len() > max {
        return Err(Error::Recording("line exceeds 16 MiB".into()));
    }
    String::from_utf8(buf)
        .map(Some)
        .map_err(|_| Error::Recording("line is not valid UTF-8".into()))
}

impl Error {
    fn with_recording_path(self, path: &Path) -> Self {
        match self {
            Self::Recording(msg) => Self::Recording(format!("{}: {msg}", path.display())),
            other => other,
        }
    }

    fn with_recording_line(self, line: usize) -> Self {
        match self {
            Self::Recording(msg) => Self::Recording(format!("line {line}: {msg}")),
            other => other,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordStatus {
    pub epoch: u64,
    pub active: bool,
    pub started_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_ms: Option<u64>,
    pub messages: u64,
    pub bytes: u64,
    pub dropped: u64,
    pub topics: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoppedRecording {
    pub temp_path: PathBuf,
    pub messages: u64,
    pub topics: u64,
    pub started_ms: u64,
    pub ended_ms: u64,
    pub dropped: u64,
    pub bytes: u64,
}

impl StoppedRecording {
    pub fn status(&self, epoch: u64) -> RecordStatus {
        RecordStatus {
            epoch,
            active: false,
            started_ms: self.started_ms,
            ended_ms: Some(self.ended_ms),
            messages: self.messages,
            bytes: self.bytes,
            dropped: self.dropped,
            topics: self.topics,
            path: Some(self.temp_path.to_string_lossy().into_owned()),
        }
    }
}

struct SharedStats {
    messages: AtomicU64,
    bytes: AtomicU64,
    topics: AtomicU64,
    dropped: AtomicU64,
}

struct WriterStats {
    messages: u64,
    bytes: u64,
    topics: u64,
}

pub struct Recorder {
    tx: Option<SyncSender<Inbound>>,
    stats: Arc<SharedStats>,
    thread: Option<JoinHandle<WriterStats>>,
    path: PathBuf,
    started_ms: u64,
}

impl Recorder {
    pub fn start(directory: impl AsRef<Path>) -> Result<Self> {
        Self::start_with_capacity(directory, RECORDER_QUEUE)
    }

    #[cfg(test)]
    fn with_channel(capacity: usize) -> (Self, mpsc::Receiver<Inbound>) {
        let (tx, rx) = mpsc::sync_channel(capacity.max(1));
        (
            Self {
                tx: Some(tx),
                stats: Arc::new(SharedStats {
                    messages: AtomicU64::new(0),
                    bytes: AtomicU64::new(0),
                    topics: AtomicU64::new(0),
                    dropped: AtomicU64::new(0),
                }),
                thread: None,
                path: PathBuf::from(".mqx-0.jsonl"),
                started_ms: 0,
            },
            rx,
        )
    }

    pub fn start_with_capacity(directory: impl AsRef<Path>, capacity: usize) -> Result<Self> {
        let directory = directory.as_ref();
        fs::create_dir_all(directory)?;
        let started_ms = now_ms();
        let path = directory.join(format!(".mqx-{started_ms}.jsonl"));
        let file = create_private_file(&path)?;
        let (tx, rx) = mpsc::sync_channel(capacity.max(1));
        let stats = Arc::new(SharedStats {
            messages: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            topics: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        });
        let thread_stats = Arc::clone(&stats);
        let thread = std::thread::Builder::new()
            .name("mqx-record".into())
            .spawn(move || writer_loop(file, rx, thread_stats))
            .map_err(|error| {
                let _ = fs::remove_file(&path);
                Error::Recording(format!("recorder thread: {error}"))
            })?;
        Ok(Self {
            tx: Some(tx),
            stats,
            thread: Some(thread),
            path,
            started_ms,
        })
    }

    pub fn try_append(&self, inbound: &Inbound) {
        let Some(tx) = self.tx.as_ref() else {
            return;
        };
        match tx.try_send(inbound.clone()) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                let dropped = self.stats.dropped.fetch_add(1, Ordering::Relaxed) + 1;
                if dropped == 1 || dropped.is_multiple_of(1000) {
                    warn!(dropped, "recorder queue full; dropping events");
                }
            }
            Err(TrySendError::Disconnected(_)) => {
                let dropped = self.stats.dropped.fetch_add(1, Ordering::Relaxed) + 1;
                if dropped == 1 || dropped.is_multiple_of(1000) {
                    warn!(dropped, "recorder writer gone; dropping events");
                }
            }
        }
    }

    pub fn status(&self, epoch: u64) -> RecordStatus {
        RecordStatus {
            epoch,
            active: true,
            started_ms: self.started_ms,
            ended_ms: None,
            messages: self.stats.messages.load(Ordering::Relaxed),
            bytes: self.stats.bytes.load(Ordering::Relaxed),
            dropped: self.stats.dropped.load(Ordering::Relaxed),
            topics: self.stats.topics.load(Ordering::Relaxed),
            path: None,
        }
    }

    pub fn stop(mut self) -> Result<StoppedRecording> {
        self.finish()
    }

    fn finish(&mut self) -> Result<StoppedRecording> {
        drop(self.tx.take());
        let writer = if let Some(thread) = self.thread.take() {
            match thread.join() {
                Ok(stats) => stats,
                Err(_) => {
                    warn!("recorder thread panicked");
                    self.snapshot_stats()
                }
            }
        } else {
            self.snapshot_stats()
        };
        Ok(StoppedRecording {
            temp_path: self.path.clone(),
            messages: writer.messages,
            topics: writer.topics,
            started_ms: self.started_ms,
            ended_ms: now_ms(),
            dropped: self.stats.dropped.load(Ordering::Relaxed),
            bytes: writer.bytes,
        })
    }

    fn snapshot_stats(&self) -> WriterStats {
        WriterStats {
            messages: self.stats.messages.load(Ordering::Relaxed),
            bytes: self.stats.bytes.load(Ordering::Relaxed),
            topics: self.stats.topics.load(Ordering::Relaxed),
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        drop(self.tx.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn writer_loop(file: File, rx: mpsc::Receiver<Inbound>, stats: Arc<SharedStats>) -> WriterStats {
    let mut out = BufWriter::new(file);
    let mut topics = HashSet::new();
    let mut messages = 0u64;
    let mut bytes = 0u64;
    while let Ok(inbound) = rx.recv() {
        let event = RecordEvent::from_inbound(&inbound);
        let line = match event.to_jsonl() {
            Ok(line) => line,
            Err(error) => {
                warn!(%error, "recorder serialize failed");
                stats.dropped.fetch_add(1, Ordering::Relaxed);
                drain_dropped(&rx, &stats);
                break;
            }
        };
        if let Err(error) = writeln!(out, "{line}") {
            warn!(%error, "recorder write failed");
            stats.dropped.fetch_add(1, Ordering::Relaxed);
            drain_dropped(&rx, &stats);
            break;
        }
        messages = messages.saturating_add(1);
        bytes = bytes.saturating_add(line.len() as u64).saturating_add(1);
        topics.insert(event.topic);
        stats.messages.store(messages, Ordering::Relaxed);
        stats.bytes.store(bytes, Ordering::Relaxed);
        stats.topics.store(topics.len() as u64, Ordering::Relaxed);
    }
    if let Err(error) = out.flush() {
        warn!(%error, "recorder flush failed");
    }
    WriterStats {
        messages,
        bytes,
        topics: topics.len() as u64,
    }
}

fn drain_dropped(rx: &mpsc::Receiver<Inbound>, stats: &SharedStats) {
    let mut extra = 0u64;
    while rx.try_recv().is_ok() {
        extra += 1;
    }
    if extra > 0 {
        stats.dropped.fetch_add(extra, Ordering::Relaxed);
    }
}

pub fn validate_recording_name(name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::Recording("recording name is empty".into()));
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(Error::Recording(
            "recording name must be a file name, not a path".into(),
        ));
    }
    let path = Path::new(name);
    if path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
    {
        return Err(Error::Recording(
            "recording name must be a file name, not a path".into(),
        ));
    }
    Ok(())
}

pub fn save_recording(
    temp_path: &Path,
    directory: &Path,
    name: &str,
    header: &RecordingHeader,
) -> Result<PathBuf> {
    validate_recording_name(name)?;
    if !is_temp_recording_path(temp_path) {
        return Err(Error::Recording("not a temporary recording".into()));
    }
    fs::create_dir_all(directory)?;
    let dest = directory.join(name);
    if dest.parent() != Some(directory) {
        return Err(Error::Recording(
            "recording name must be a file name, not a path".into(),
        ));
    }
    {
        let mut src = File::open(temp_path)?;
        let mut dest_file = BufWriter::new(create_exclusive_private_file(&dest)?);
        writeln!(dest_file, "{}", header.to_jsonl()?)?;
        std::io::copy(&mut src, &mut dest_file)?;
        dest_file.flush()?;
    }
    fs::remove_file(temp_path)?;
    Ok(dest)
}

pub fn discard_recording(temp_path: &Path) -> Result<()> {
    if !is_temp_recording_path(temp_path) {
        return Err(Error::Recording("not a temporary recording".into()));
    }
    match fs::remove_file(temp_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn list_recording_files(dir: &Path) -> Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    !name.starts_with('.') && name.ends_with(".jsonl") && path.is_file()
                })
        })
        .collect();
    files.sort();
    Ok(files)
}

pub fn unix_ms_to_rfc3339(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let millis = (ms % 1000) as u32;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400) as u32;
    let hour = tod / 3600;
    let min = (tod % 3600) / 60;
    let sec = tod % 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}.{millis:03}Z")
}

fn is_temp_recording_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(".mqx-") && name.ends_with(".jsonl"))
}

fn create_private_file(path: &Path) -> Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        Ok(File::create(path)?)
    }
}

fn create_exclusive_private_file(path: &Path) -> Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(path)
            .map_err(|error| already_exists_or(error, path))?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|error| already_exists_or(error, path))
    }
}

fn already_exists_or(error: std::io::Error, path: &Path) -> Error {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file");
        Error::Recording(format!("a recording named {name} already exists"))
    } else {
        error.into()
    }
}

fn now_ms() -> u64 {
    unix_ms(std::time::SystemTime::now())
}

fn unix_ms(time: std::time::SystemTime) -> u64 {
    time.duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Unix days since 1970-01-01 → UTC civil date (Howard Hinnant).
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        io::Write,
        path::{Path, PathBuf},
        time::{Duration, SystemTime},
    };

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    #[test]
    fn binary_payload_roundtrip() {
        let event = RecordEvent {
            t_ms: 1_700_000_000_123,
            topic: "home/lamp".into(),
            qos: 1,
            retain: true,
            dup: false,
            payload: BASE64.encode([0_u8, 1, 2, 255]),
        };
        let line = event.to_jsonl().unwrap();
        let parsed = RecordEvent::from_jsonl(&line).unwrap();
        assert_eq!(parsed, event);
        assert_eq!(parsed.payload_bytes().unwrap(), vec![0, 1, 2, 255]);
    }

    #[test]
    fn header_camel_case_fixture() {
        let (header, events) = load_recording(fixture("header.jsonl")).unwrap();
        let header = header.expect("header");
        assert_eq!(header.kind, RECORDING_KIND);
        assert_eq!(header.v, 1);
        assert_eq!(header.started_at, "2026-08-26T12:00:00.000Z");
        assert_eq!(header.ended_at, "2026-08-26T12:00:32.100Z");
        assert_eq!(header.profile_id, "abc");
        assert_eq!(header.profile_name, "Home Assistant");
        assert_eq!(header.broker, "mqtts://ha.local:8883");
        assert_eq!(header.messages, 2);
        assert_eq!(header.topics, 1);
        assert_eq!(header.app_version, "0.2.0");
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].payload_bytes().unwrap(), vec![0, 1, 2, 255]);

        let json = header.to_jsonl().unwrap();
        for key in [
            "startedAt",
            "endedAt",
            "profileId",
            "profileName",
            "appVersion",
        ] {
            assert!(json.contains(key), "missing {key} in {json}");
        }
        for key in [
            "started_at",
            "ended_at",
            "profile_id",
            "profile_name",
            "app_version",
        ] {
            assert!(!json.contains(key), "snake_case leaked: {json}");
        }
    }

    #[test]
    fn headerless_parse() {
        let (header, events) = load_recording(fixture("headerless.jsonl")).unwrap();
        assert!(header.is_none());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].topic, "home/lamp");
        assert!(!events[0].dup);
        assert_eq!(events[0].payload_bytes().unwrap(), b"{\"on\":true}");
    }

    #[test]
    fn sniff_kind_is_header() {
        let line = r#"{"kind":"mqx-recording","v":1,"startedAt":"2026-08-26T12:00:00.000Z","endedAt":"2026-08-26T12:00:32.100Z","profileId":"p","profileName":"n","broker":"mqtt://localhost:1883","messages":0,"topics":0,"appVersion":"0.2.0"}"#;
        assert!(matches!(
            sniff_line(line).unwrap(),
            RecordingLine::Header(_)
        ));
        let event = r#"{"t_ms":1,"topic":"t","qos":0,"retain":false,"payload":""}"#;
        assert!(matches!(
            sniff_line(event).unwrap(),
            RecordingLine::Event(_)
        ));
    }

    #[test]
    fn realtime_waits_recorded_gap() {
        let wait = wait_duration(1_000, 1_250, 1.0, Duration::from_millis(50));
        assert_eq!(wait, Duration::from_millis(200));
    }

    #[test]
    fn double_speed_halves_wait() {
        let wait = wait_duration(0, 1_000, 2.0, Duration::ZERO);
        assert_eq!(wait, Duration::from_millis(500));
    }

    #[test]
    fn max_speed_does_not_wait() {
        assert_eq!(wait_duration(0, 5_000, 0.0, Duration::ZERO), Duration::ZERO);
    }

    #[test]
    fn late_clock_does_not_sleep() {
        assert_eq!(
            wait_duration(0, 100, 1.0, Duration::from_millis(150)),
            Duration::ZERO
        );
    }

    #[test]
    fn capped_line_rejects_over_max() {
        let mut reader = BufReader::new(&b"abcdef\n"[..]);
        let err = read_line_capped(&mut reader, 4).unwrap_err();
        assert!(matches!(err, Error::Recording(_)));
    }

    #[test]
    fn capped_line_accepts_crlf_at_max() {
        let mut reader = BufReader::new(&b"abcd\r\n"[..]);
        let line = read_line_capped(&mut reader, 4).unwrap().unwrap();
        assert_eq!(line, "abcd");
    }

    #[test]
    fn capped_line_rejects_over_max_crlf() {
        let mut reader = BufReader::new(&b"abcde\r\n"[..]);
        let err = read_line_capped(&mut reader, 4).unwrap_err();
        assert!(matches!(err, Error::Recording(_)));
    }

    #[test]
    fn load_recording_skips_blank_lines() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(
            concat!(
                "\n",
                r#"{"kind":"mqx-recording","v":1,"startedAt":"2026-08-26T12:00:00.000Z","endedAt":"2026-08-26T12:00:01.000Z","profileId":"p","profileName":"n","broker":"mqtt://localhost:1883","messages":1,"topics":1,"appVersion":"0.2.0"}"#,
                "\n\n",
                r#"{"t_ms":1,"topic":"t","qos":0,"retain":false,"payload":"AA=="}"#,
                "\n",
            )
            .as_bytes(),
        )
        .unwrap();
        tmp.flush().unwrap();
        let (header, events) = load_recording(tmp.path()).unwrap();
        assert!(header.is_some());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].topic, "t");
    }

    fn sample_header() -> RecordingHeader {
        RecordingHeader {
            kind: RECORDING_KIND.into(),
            v: 1,
            started_at: "2026-08-26T12:00:00.000Z".into(),
            ended_at: "2026-08-26T12:00:32.100Z".into(),
            profile_id: "p".into(),
            profile_name: "n".into(),
            broker: "mqtt://localhost:1883".into(),
            messages: 0,
            topics: 0,
            app_version: "0.2.0".into(),
        }
    }

    fn inbound(topic: &str) -> Inbound {
        Inbound {
            topic: topic.into(),
            payload: bytes::Bytes::from_static(b"on"),
            retain: false,
            qos: crate::message::QoS::AtMostOnce,
            dup: false,
            timestamp: std::time::SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn save_recording_rejects_parent_path() {
        assert!(validate_recording_name("../x.jsonl").is_err());
        assert!(validate_recording_name("a/b.jsonl").is_err());
        assert!(validate_recording_name("").is_err());
        assert!(validate_recording_name("..").is_err());
        assert!(validate_recording_name("ok.jsonl").is_ok());

        let dir = tempfile::tempdir().unwrap();
        let temp = dir.path().join(".mqx-1.jsonl");
        std::fs::write(
            &temp,
            b"{\"t_ms\":1,\"topic\":\"t\",\"qos\":0,\"retain\":false,\"payload\":\"\"}\n",
        )
        .unwrap();
        let err = save_recording(&temp, dir.path(), "../x.jsonl", &sample_header()).unwrap_err();
        assert!(matches!(err, Error::Recording(_)));
        assert!(temp.exists());
    }

    #[test]
    fn drop_on_full_increments_dropped() {
        let (recorder, _rx) = Recorder::with_channel(1);
        recorder.try_append(&inbound("a"));
        recorder.try_append(&inbound("b"));
        recorder.try_append(&inbound("c"));
        assert_eq!(recorder.stats.dropped.load(Ordering::Relaxed), 2);
        assert_eq!(recorder.status(1).dropped, 2);
    }

    #[test]
    fn disconnected_increments_dropped() {
        let (recorder, rx) = Recorder::with_channel(4);
        drop(rx);
        recorder.try_append(&inbound("a"));
        recorder.try_append(&inbound("b"));
        assert_eq!(recorder.status(1).dropped, 2);
    }

    #[test]
    fn stop_returns_partial_after_writer_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mqx-err.jsonl");
        std::fs::write(&path, b"").unwrap();
        let stats = Arc::new(SharedStats {
            messages: AtomicU64::new(4),
            bytes: AtomicU64::new(10),
            topics: AtomicU64::new(2),
            dropped: AtomicU64::new(1),
        });
        let recorder = Recorder {
            tx: None,
            stats,
            thread: Some(std::thread::spawn(|| panic!("disk full"))),
            path: path.clone(),
            started_ms: 1,
        };
        let stopped = recorder.stop().unwrap();
        assert_eq!(stopped.messages, 4);
        assert_eq!(stopped.topics, 2);
        assert_eq!(stopped.dropped, 1);
        assert_eq!(stopped.temp_path, path);
        assert!(path.exists());
    }

    #[test]
    fn save_recording_prepends_header_and_skips_dotfiles() {
        let dir = tempfile::tempdir().unwrap();
        let temp = dir.path().join(".mqx-2.jsonl");
        std::fs::write(
            &temp,
            b"{\"t_ms\":1,\"topic\":\"t\",\"qos\":0,\"retain\":false,\"payload\":\"\"}\n",
        )
        .unwrap();
        let dest = save_recording(&temp, dir.path(), "lamp.jsonl", &sample_header()).unwrap();
        assert!(!temp.exists());
        let (header, events) = load_recording(&dest).unwrap();
        assert_eq!(header.expect("header").kind, RECORDING_KIND);
        assert_eq!(events.len(), 1);
        let listed = list_recording_files(dir.path()).unwrap();
        assert_eq!(listed, vec![dest]);
        let leftover = dir.path().join(".mqx-hidden.jsonl");
        std::fs::write(&leftover, b"{}\n").unwrap();
        let listed = list_recording_files(dir.path()).unwrap();
        assert_eq!(listed.len(), 1);
    }

    #[test]
    fn save_recording_does_not_overwrite_existing() {
        let dir = tempfile::tempdir().unwrap();
        let first_temp = dir.path().join(".mqx-3.jsonl");
        let second_temp = dir.path().join(".mqx-4.jsonl");
        std::fs::write(
            &first_temp,
            b"{\"t_ms\":1,\"topic\":\"a\",\"qos\":0,\"retain\":false,\"payload\":\"\"}\n",
        )
        .unwrap();
        std::fs::write(
            &second_temp,
            b"{\"t_ms\":2,\"topic\":\"b\",\"qos\":0,\"retain\":false,\"payload\":\"\"}\n",
        )
        .unwrap();
        let dest = save_recording(&first_temp, dir.path(), "lamp.jsonl", &sample_header()).unwrap();
        let err =
            save_recording(&second_temp, dir.path(), "lamp.jsonl", &sample_header()).unwrap_err();
        assert!(matches!(err, Error::Recording(message) if message.contains("already exists")));
        assert!(second_temp.exists());
        let (_, events) = load_recording(&dest).unwrap();
        assert_eq!(events[0].topic, "a");
    }

    #[test]
    fn unix_ms_to_rfc3339_epoch() {
        assert_eq!(unix_ms_to_rfc3339(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(unix_ms_to_rfc3339(1), "1970-01-01T00:00:00.001Z");
    }

    #[cfg(unix)]
    #[test]
    fn temp_recording_is_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let recorder = Recorder::start(dir.path()).unwrap();
        let mode = std::fs::metadata(&recorder.path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        recorder.stop().unwrap();
    }

    #[test]
    fn header_v2_is_refused_before_event_lines() {
        let data = concat!(
            r#"{"kind":"mqx-recording","v":2,"startedAt":"2026-08-26T12:00:00.000Z","endedAt":"2026-08-26T12:00:01.000Z","profileId":"p","profileName":"n","broker":"mqtt://localhost:1883","messages":1,"topics":1,"appVersion":"9.0.0"}"#,
            "\n",
            "this is not an event\n",
        );
        let err = load_recording_from(data.as_bytes()).unwrap_err();
        assert!(
            matches!(err, Error::Recording(ref msg) if msg.contains("upgrade mqx")),
            "{err}"
        );
    }

    fn write_jsonl(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn files_larger_than_4_gib_are_refused() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.as_file().set_len(MAX_RECORDING_BYTES + 1).unwrap();
        let err = load_replay_events(tmp.path()).unwrap_err();
        assert!(
            matches!(err, Error::Recording(ref msg) if msg.contains("4 GiB")),
            "{err}"
        );
    }

    #[test]
    fn load_replay_events_skips_header() {
        let events = load_replay_events(fixture("header.jsonl")).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].topic, "home/lamp");
    }

    #[test]
    fn inspect_header_uses_header_stats() {
        let path = fixture("header.jsonl");
        let meta = fs::metadata(&path).unwrap();
        let info = inspect_recording(&path, &meta).unwrap();
        assert_eq!(info.name, "header");
        assert_eq!(info.started_at.as_deref(), Some("2026-08-26T12:00:00.000Z"));
        assert_eq!(info.messages, 2);
        assert_eq!(info.topics, 1);
        assert!(info.first_t_ms.is_none());
    }

    #[test]
    fn inspect_headerless_scans_counts() {
        let path = fixture("headerless.jsonl");
        let meta = fs::metadata(&path).unwrap();
        let info = inspect_recording(&path, &meta).unwrap();
        assert_eq!(info.messages, 1);
        assert_eq!(info.topics, 1);
        assert_eq!(info.first_t_ms, Some(1_787_735_165_782));
        assert!(info.started_at.is_none());
    }

    #[test]
    fn list_recordings_skips_dotfiles_and_non_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        write_jsonl(
            dir.path(),
            "keep.jsonl",
            "{\"t_ms\":1,\"topic\":\"a\",\"qos\":0,\"retain\":false,\"payload\":\"YQ==\"}\n",
        );
        write_jsonl(
            dir.path(),
            ".mqx-1.jsonl",
            "{\"t_ms\":1,\"topic\":\"hidden\",\"qos\":0,\"retain\":false,\"payload\":\"YQ==\"}\n",
        );
        fs::write(dir.path().join("notes.txt"), "nope").unwrap();
        let mut cache = RecordingScanCache::default();
        let list = list_recordings(dir.path(), &mut cache).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "keep");
        assert_eq!(list[0].messages, 1);
    }

    #[test]
    fn headerless_scan_cached_until_mtime_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_jsonl(
            dir.path(),
            "trace.jsonl",
            "{\"t_ms\":1,\"topic\":\"a\",\"qos\":0,\"retain\":false,\"payload\":\"YQ==\"}\n",
        );
        let mut cache = RecordingScanCache::default();
        let first = list_recordings(dir.path(), &mut cache).unwrap();
        assert_eq!(first[0].messages, 1);
        assert_eq!(cache.entries.len(), 1);

        fs::write(
            &path,
            concat!(
                "{\"t_ms\":1,\"topic\":\"a\",\"qos\":0,\"retain\":false,\"payload\":\"YQ==\"}\n",
                "{\"t_ms\":2,\"topic\":\"b\",\"qos\":0,\"retain\":false,\"payload\":\"Yg==\"}\n",
            ),
        )
        .unwrap();
        let mtime = SystemTime::now() + Duration::from_secs(2);
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
        let second = list_recordings(dir.path(), &mut cache).unwrap();
        assert_eq!(second[0].messages, 2);
        assert_eq!(second[0].topics, 2);
    }
}
