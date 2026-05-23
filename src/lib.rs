#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("empty line")]
    EmptyLine,
    #[error("invalid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("invalid logfmt: {0}")]
    InvalidLogfmt(String),
    #[error("JSON value must be an object")]
    NotObject,
}

#[derive(Debug, Error)]
pub enum RouterError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid output path")]
    InvalidPath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputFormat {
    Jsonl,
    Logfmt,
    Auto,
}

impl InputFormat {
    pub fn parse_arg(s: &str) -> Option<Self> {
        match s {
            "jsonl" => Some(Self::Jsonl),
            "logfmt" => Some(Self::Logfmt),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }
}

pub fn parse_jsonl_line(line: &str) -> Result<(), ParseError> {
    let trimmed = line.trim_end_matches(['\r', '\n']);
    if trimmed.is_empty() {
        return Err(ParseError::EmptyLine);
    }
    let value: Value = serde_json::from_str(trimmed)?;
    if !value.is_object() {
        return Err(ParseError::NotObject);
    }
    Ok(())
}

pub fn parse_logfmt_line(line: &str) -> Result<HashMap<String, String>, ParseError> {
    let trimmed = line.trim_end_matches(['\r', '\n']);
    if trimmed.is_empty() {
        return Err(ParseError::EmptyLine);
    }
    parse_logfmt(trimmed).map_err(ParseError::InvalidLogfmt)
}

pub fn parse_auto_line(line: &str) -> Result<(), ParseError> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err(ParseError::EmptyLine);
    }
    if trimmed.starts_with('{') {
        parse_jsonl_line(line)
    } else {
        parse_logfmt_line(line).map(|_| ())
    }
}

fn parse_logfmt(input: &str) -> Result<HashMap<String, String>, String> {
    let mut fields = HashMap::new();
    let mut rest = input.trim();
    while !rest.is_empty() {
        let (key, value, remaining) = parse_logfmt_pair(rest)?;
        fields.insert(key, value);
        rest = remaining.trim_start();
    }
    Ok(fields)
}

fn parse_logfmt_pair(input: &str) -> Result<(String, String, &str), String> {
    let eq = input
        .find('=')
        .ok_or_else(|| "missing '=' in logfmt pair".to_string())?;
    let key = input[..eq].trim();
    if key.is_empty() {
        return Err("empty logfmt key".to_string());
    }
    let after_eq = &input[eq + 1..];
    let (value, remaining) = parse_logfmt_value(after_eq)?;
    Ok((key.to_string(), value, remaining))
}

fn parse_logfmt_value(input: &str) -> Result<(String, &str), String> {
    if let Some(rest) = input.strip_prefix('"') {
        let mut escaped = false;
        let mut end = 0;
        for (i, ch) in rest.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                end = i;
                break;
            }
        }
        if end == 0 && !rest.contains('"') {
            return Err("unterminated quoted logfmt value".to_string());
        }
        let raw = &rest[..end];
        let value = unescape_logfmt(raw);
        let remaining = &rest[end + 1..];
        return Ok((value, remaining));
    }

    let end = input
        .find(char::is_whitespace)
        .unwrap_or(input.len());
    Ok((input[..end].to_string(), &input[end..]))
}

fn unescape_logfmt(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(ch);
        }
    }
    out
}

pub fn field_value_from_json_line(line: &str, field: &str) -> Option<String> {
    let trimmed = line.trim_end_matches(['\r', '\n']);
    let value: Value = serde_json::from_str(trimmed).ok()?;
    let obj = value.as_object()?;
    value_to_string(obj.get(field)?)
}

pub fn field_value_from_logfmt_line(line: &str, field: &str) -> Option<String> {
    let trimmed = line.trim_end_matches(['\r', '\n']);
    let map = parse_logfmt(trimmed).ok()?;
    map.get(field).cloned()
}

pub fn field_value_auto(line: &str, field: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.starts_with('{') {
        field_value_from_json_line(line, field)
    } else {
        field_value_from_logfmt_line(line, field)
    }
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Null => Some("null".to_string()),
        _ => None,
    }
}

fn sanitize_filename(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch == '/' || ch == '\\' {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    if out.is_empty() {
        "__empty__".to_string()
    } else {
        out
    }
}

pub struct Router {
    out_dir: PathBuf,
    by_field: String,
    writers: HashMap<String, BufWriter<File>>,
}

impl Router {
    pub fn new(out_dir: PathBuf, by_field: String) -> Result<Self, RouterError> {
        fs::create_dir_all(&out_dir)?;
        Ok(Self {
            out_dir,
            by_field,
            writers: HashMap::new(),
        })
    }

    pub fn emit_jsonl_line(&mut self, line: &str) -> Result<(), RouterError> {
        let key = field_value_from_json_line(line, &self.by_field)
            .unwrap_or_else(|| "__missing__".to_string());
        self.write_line(&key, line)
    }

    pub fn emit_logfmt_line(&mut self, line: &str) -> Result<(), RouterError> {
        let key = field_value_from_logfmt_line(line, &self.by_field)
            .unwrap_or_else(|| "__missing__".to_string());
        self.write_line(&key, line)
    }

    pub fn emit_auto_line(&mut self, line: &str) -> Result<(), RouterError> {
        let key = field_value_auto(line, &self.by_field)
            .unwrap_or_else(|| "__missing__".to_string());
        self.write_line(&key, line)
    }

    fn write_line(&mut self, key: &str, line: &str) -> Result<(), RouterError> {
        let file_name = format!("{}.log", sanitize_filename(key));
        let path = self.out_dir.join(&file_name);
        if path.file_name().is_none() {
            return Err(RouterError::InvalidPath);
        }

        let writer = if let Some(writer) = self.writers.get_mut(key) {
            writer
        } else {
            let file = OpenOptions::new().create(true).append(true).open(&path)?;
            let writer = BufWriter::new(file);
            self.writers.insert(key.to_string(), writer);
            self.writers
                .get_mut(key)
                .ok_or(RouterError::InvalidPath)?
        };

        writer.write_all(line.as_bytes())?;
        if !line.ends_with('\n') {
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        Ok(())
    }

    pub fn flush_all(&mut self) -> Result<(), RouterError> {
        for writer in self.writers.values_mut() {
            writer.flush()?;
        }
        Ok(())
    }
}

pub fn process_lines<I>(
    lines: I,
    format: InputFormat,
    router: &mut Router,
    strict_parse: bool,
) -> Result<bool, RouterError>
where
    I: Iterator<Item = Result<String, io::Error>>,
{
    let mut had_parse_errors = false;
    for line_result in lines {
        let line = line_result?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parse_result = match format {
            InputFormat::Jsonl => parse_jsonl_line(&line).map(|_| ()),
            InputFormat::Logfmt => parse_logfmt_line(&line).map(|_| ()),
            InputFormat::Auto => parse_auto_line(&line),
        };

        if let Err(_err) = parse_result {
            if strict_parse {
                had_parse_errors = true;
            }
            continue;
        }

        match format {
            InputFormat::Jsonl => router.emit_jsonl_line(&line)?,
            InputFormat::Logfmt => router.emit_logfmt_line(&line)?,
            InputFormat::Auto => router.emit_auto_line(&line)?,
        }
    }
    router.flush_all()?;
    Ok(had_parse_errors)
}

pub fn open_input_paths(paths: &[PathBuf]) -> Result<Vec<File>, io::Error> {
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        files.push(File::open(path)?);
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_jsonl_valid() {
        assert!(parse_jsonl_line(r#"{"a":1}"#).is_ok());
    }

    #[test]
    fn parse_jsonl_rejects_non_object() {
        assert!(matches!(
            parse_jsonl_line("[1]"),
            Err(ParseError::NotObject)
        ));
    }

    #[test]
    fn parse_logfmt_extracts_fields() {
        let map = parse_logfmt_line("level=info msg=hello").expect("logfmt");
        assert_eq!(map.get("level").map(String::as_str), Some("info"));
        assert_eq!(map.get("msg").map(String::as_str), Some("hello"));
    }
}
