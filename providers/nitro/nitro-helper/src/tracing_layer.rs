//! Copyright (c) 2019 Tokio Contributors (licensed under the MIT License)
//! Modifications Copyright (c) 2021, Foris Limited (licensed under the Apache License, Version 2.0)

use std::collections::HashMap;
use std::{fmt, io::Write};
use tracing_core::{
    event::Event,
    field::Visit,
    span::{Attributes, Id, Record},
    Field, Level, Metadata, Subscriber,
};
use tracing_subscriber::{layer::Context, registry::LookupSpan};
use vsock::{SockAddr, VsockStream};

pub struct Layer {
    cid: u32,
    local_port: u32,
    field_prefix: Option<String>,
}

impl Layer {
    pub fn new(cid: u32, local_port: u32) -> Self {
        Self {
            cid,
            local_port,
            field_prefix: None,
        }
    }

    pub fn get_socket(&self) -> Result<VsockStream, String> {
        let addr = SockAddr::new_vsock(self.cid, self.local_port);
        VsockStream::connect(&addr).map_err(|e| {
            format!(
                "failed to connect to the enclave server to push log: {:?}",
                e
            )
        })
    }

    /// Sets the prefix to apply to names of user-defined fields other than the event `message`
    /// field. Defaults to `Some("F")`.
    pub fn with_field_prefix(mut self, x: Option<String>) -> Self {
        self.field_prefix = x;
        self
    }
}

impl<S> tracing_subscriber::Layer<S> for Layer
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    fn new_span(&self, attrs: &Attributes, id: &Id, ctx: Context<S>) {
        let span = ctx.span(id).expect("unknown span");
        let mut buf = Vec::with_capacity(256);

        let depth = span.parents().count();

        writeln!(buf, "S{}_NAME", depth).unwrap();
        put_value(&mut buf, span.name().as_bytes());
        put_metadata(&mut buf, span.metadata(), Some(depth));

        attrs.record(&mut SpanVisitor {
            buf: &mut buf,
            depth,
            prefix: self.field_prefix.as_ref().map(|x| &x[..]),
        });

        span.extensions_mut().insert(SpanFields(buf));
    }

    fn on_record(&self, id: &Id, values: &Record, ctx: Context<S>) {
        let span = ctx.span(id).expect("unknown span");
        let depth = span.parents().count();
        let mut exts = span.extensions_mut();
        let buf = &mut exts.get_mut::<SpanFields>().expect("missing fields").0;
        values.record(&mut SpanVisitor {
            buf,
            depth,
            prefix: self.field_prefix.as_ref().map(|x| &x[..]),
        });
    }

    fn on_event(&self, event: &Event, ctx: Context<S>) {
        let mut buf = Vec::with_capacity(256);

        // Record span fields
        for span in ctx.scope() {
            let exts = span.extensions();
            let fields = exts.get::<SpanFields>().expect("missing fields");
            buf.extend_from_slice(&fields.0);
        }

        // Record event fields
        put_metadata(&mut buf, event.metadata(), None);
        event.record(&mut EventVisitor::new(
            &mut buf,
            self.field_prefix.as_ref().map(|x| &x[..]),
        ));

        if let Ok(socket) = &mut self.get_socket() {
            // add \n at the end of the buffer
            if let Err(e) = socket.write_all(&buf) {
                eprintln!("{:?}", e);
            }
        }
    }
}

struct SpanFields(Vec<u8>);

struct SpanVisitor<'a> {
    buf: &'a mut Vec<u8>,
    depth: usize,
    prefix: Option<&'a str>,
}

impl Visit for SpanVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        write!(self.buf, "S{}", self.depth).unwrap();
        if let Some(prefix) = self.prefix {
            self.buf.extend_from_slice(prefix.as_bytes());
        }
        self.buf.push(b'_');
        put_debug(self.buf, field.name(), value);
    }
}

/// Helper for generating the journal export format, which is consumed by journald:
/// https://www.freedesktop.org/wiki/Software/systemd/export/
struct EventVisitor<'a> {
    buf: &'a mut Vec<u8>,
    prefix: Option<&'a str>,
}

impl<'a> EventVisitor<'a> {
    fn new(buf: &'a mut Vec<u8>, prefix: Option<&'a str>) -> Self {
        Self { buf, prefix }
    }
}

impl Visit for EventVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if let Some(prefix) = self.prefix {
            if field.name() != "message" {
                self.buf.extend_from_slice(prefix.as_bytes());
                self.buf.push(b'_');
            }
        }
        put_debug(self.buf, field.name(), value);
    }
}

fn put_metadata(buf: &mut Vec<u8>, meta: &Metadata, span: Option<usize>) {
    let raw_level = get_raw_level(meta.level());
    if span.is_none() {
        put_field(buf, "PRIORITY", raw_level);
    }
    if let Some(n) = span {
        write!(buf, "S{}_", n).unwrap();
    }
    put_field(buf, "TARGET", meta.target().as_bytes());
    if let Some(file) = meta.file() {
        if let Some(n) = span {
            write!(buf, "S{}_", n).unwrap();
        }
        put_field(buf, "CODE_FILE", file.as_bytes());
    }
    if let Some(line) = meta.line() {
        if let Some(n) = span {
            write!(buf, "S{}_", n).unwrap();
        }
        // Text format is safe as a line number can't possibly contain anything funny
        put_field(buf, "CODE_LINE", line.to_string().as_bytes());
    }
}

fn put_debug(buf: &mut Vec<u8>, name: &str, value: &dyn fmt::Debug) {
    sanitize_name(name, buf);
    buf.push(b'\n');
    buf.extend_from_slice(&[0; 8]); // Length tag, to be populated
    let start = buf.len();
    write!(buf, "{:?}", value).unwrap();
    let end = buf.len();
    buf[start - 8..start].copy_from_slice(&((end - start) as u64).to_le_bytes());
    buf.push(b'\n');
}

/// Mangle a name into journald-compliant form
fn sanitize_name(name: &str, buf: &mut Vec<u8>) {
    buf.extend(
        name.bytes()
            .map(|c| if c == b'.' { b'_' } else { c })
            .skip_while(|&c| c == b'_')
            .filter(|&c| c == b'_' || char::from(c).is_ascii_alphanumeric())
            .map(|c| char::from(c).to_ascii_uppercase() as u8),
    );
}

/// Append arbitrary data with a well-formed name
fn put_field(buf: &mut Vec<u8>, name: &str, value: &[u8]) {
    buf.extend_from_slice(name.as_bytes());
    buf.push(b'\n');
    put_value(buf, value);
}

/// Write the value portion of a key-value pair
fn put_value(buf: &mut Vec<u8>, value: &[u8]) {
    buf.extend_from_slice(&(value.len() as u64).to_le_bytes());
    buf.extend_from_slice(value);
    buf.push(b'\n');
}

fn get_log_level(raw_level: &[u8; 1]) -> Level {
    match raw_level {
        b"3" => Level::ERROR,
        b"4" => Level::WARN,
        b"5" => Level::INFO,
        b"6" => Level::DEBUG,
        b"7" => Level::TRACE,
        _ => unreachable!(),
    }
}

fn get_raw_level(level: &Level) -> &[u8; 1] {
    match *level {
        Level::ERROR => b"3",
        Level::WARN => b"4",
        Level::INFO => b"5",
        Level::DEBUG => b"6",
        Level::TRACE => b"7",
    }
}

#[derive(Clone, Debug)]
pub struct Log {
    pub level: Level,
    pub target: String,
    pub code_file: String,
    pub code_line: u32,
    pub message: String,
    pub debug: HashMap<String, String>,
}

// '\n' turn out to be 10 in raw
static FLAG: u8 = 10;

impl Default for Log {
    fn default() -> Self {
        Self {
            level: Level::TRACE,
            target: "".into(),
            code_file: "".into(),
            code_line: 0,
            message: "".into(),
            debug: HashMap::new(),
        }
    }
}

impl Log {
    pub fn format(&self) -> String {
        let mut s = format!(
            "[{}] {}:{} {}",
            self.target, self.code_file, self.code_line, self.message
        );
        for (k, v) in self.debug.iter() {
            s = format!("{}, {}={}", s, k, v);
        }
        s
    }

    pub fn from_raw(raw: &[u8]) -> std::io::Result<Self> {
        let mut flag_index_next = 0;
        let len = raw.len();
        let mut log = Log::default();
        loop {
            if flag_index_next >= len {
                break;
            }
            if let Some(ref i) = &raw[flag_index_next..len].iter().position(|x| x == &FLAG) {
                if i >= &(len - 1) {
                    break;
                }
                let index_key_begin = flag_index_next;
                let index_key_end = index_key_begin + i;
                let key_raw = &raw[index_key_begin..index_key_end];

                let index_value_len_begin = index_key_end + 1;
                let index_value_len_end = index_value_len_begin + 8;

                let mut value_len_raw = [0; 8];
                value_len_raw.copy_from_slice(&raw[index_value_len_begin..index_value_len_end]);
                let value_len = u64::from_le_bytes(value_len_raw) as usize;

                let index_value_begin = index_value_len_end;
                let index_value_end = index_value_begin + value_len;
                flag_index_next = index_value_end + 1;
                let value_raw = &raw[index_value_begin..index_value_end];
                log.update(key_raw, value_raw);
            } else {
                break;
            }
        }
        Ok(log)
    }

    fn update(&mut self, key_raw: &[u8], value_raw: &[u8]) {
        let key = String::from_utf8(key_raw.to_vec()).unwrap();
        match key.as_str() {
            "PRIORITY" => {
                let mut v = [0; 1];
                v.copy_from_slice(value_raw);
                let value = get_log_level(&v);
                self.level = value;
            }
            "MESSAGE" => {
                let value = String::from_utf8(value_raw.to_vec()).unwrap();
                self.message = value;
            }
            "TARGET" => {
                let value = String::from_utf8(value_raw.to_vec()).unwrap();
                self.target = value;
            }
            "CODE_FILE" => {
                let value = String::from_utf8(value_raw.to_vec()).unwrap();
                self.code_file = value;
            }
            "CODE_LINE" => {
                let value = String::from_utf8(value_raw.to_vec()).unwrap();
                self.code_line = value.parse().unwrap();
            }
            _ => {
                let value = String::from_utf8(value_raw.to_vec()).unwrap();
                self.debug.insert(key.to_lowercase(), value);
            }
        }
    }
}
