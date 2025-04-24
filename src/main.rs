// Copyright 2019 The Chromium Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

mod path;
mod wasm;

use fallible_iterator::FallibleIterator;
use gimli::{constants, ColumnType, Dwarf, EndianSlice, LittleEndian, Reader, ReaderOffset};
use indexmap::IndexMap;
use path::Path;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::convert::TryInto;
use std::io::Read;
use std::rc::Rc;
use std::{error, fmt};
use wasm::{parse_sections, ResolverError, SectionKind};

#[derive(Debug, PartialOrd, Ord, PartialEq, Eq, Clone, Copy)]
pub struct Pos {
  line: u32,
  column: u32,
}

#[derive(Debug, Clone)]
struct LocationEntry {
  addr: u64,
  pos: Pos,
}

pub struct FileEntries {
  language: u16,
  entries: Vec<LocationEntry>,
}

enum FuncState {
  Start,
  Ignored,
  Normal,
}

#[derive(Default)]
pub struct Resolver {
  locations: Vec<LocationEntry>,
  locations_by_filename: IndexMap<Rc<String>, FileEntries>,
}

impl Resolver {
  pub fn new<R: Reader + Clone + Default>(src: R) -> Result<Resolver, ResolverError> {
    let mut code_section_offset = None;
    let mut sections = HashMap::new();

    for section in parse_sections(src.clone())?.iterator() {
      let section = section?;

      match section.kind {
        SectionKind::Custom { name } => {
          if name.starts_with(".debug_") {
            sections.insert(name, section.payload);
          }
        }
        SectionKind::Standard { id } if id.get() == 10 => {
          code_section_offset = Some(section.payload.offset_from(&src));
        }
        _ => {}
      }
    }

    let code_section_offset: u64 = code_section_offset
      .ok_or_else(|| ResolverError::MissingCodeSection)?
      .into_u64()
      .try_into()
      .unwrap();

    let dwarf = Dwarf::load::<_, _, ResolverError>(
      |id| Ok(sections.get(id.name()).cloned().unwrap_or_default()),
      |_| Ok(Default::default()),
    )?;

    let mut res = Self::default();

    let mut iter = dwarf.units();

    while let Some(unit) = iter.next()? {
      let mut unit = dwarf.unit(unit)?;

      let line_program = match unit.line_program.take() {
        Some(line_program) => line_program,
        None => continue,
      };

      let lang = {
        let mut entries = unit.entries();
        entries.next_entry()?;
        match entries
          .current()
          .unwrap()
          .attr_value(gimli::DW_AT_language)?
        {
          Some(gimli::AttributeValue::Language(lang)) => lang.0,
          _ => 0,
        }
      };

      let is_rust = lang == constants::DW_LANG_Rust.0;

      let mut rows = line_program.rows();

      let mut func_state = FuncState::Start;

      while let Some((header, row)) = rows.next_row()? {
        if let FuncState::Start = func_state {
          func_state = if row.address() == 0 {
            FuncState::Ignored
          } else {
            FuncState::Normal
          };
        }

        if let FuncState::Ignored = func_state {
          if row.end_sequence() {
            func_state = FuncState::Start;
          }
          continue;
        }

        let file = match row.file(header) {
          Some(file) => file,
          None => continue,
        };

        let pos = {
          let line = match row.line() {
            Some(line) => line.checked_sub(1).unwrap().try_into().unwrap(),
            None => continue, // couldn't attribute instruction to any line
          };

          let column = match row.column() {
            ColumnType::Column(mut column) => {
              // DWARF columns are 1-based, Source Map are 0-based.
              column -= 1;
              // ...but Rust doesn't implement DWARF columns correctly
              // (see https://github.com/rust-lang/rust/issues/65437)
              if is_rust {
                column += 1;
              }
              column.try_into().unwrap()
            }
            ColumnType::LeftEdge => 0,
          };
          Pos { line, column }
        };

        let addr: u64 = row.address().try_into().unwrap();

        // let mut path = unit_dir.borrow();
        let mut path = Path::new(Cow::from("."));

        let dir_value;
        if let Some(dir) = file.directory(header) {
          dir_value = dwarf.attr_string(&unit, dir)?;
          path = Path::new(Cow::from(dir_value.to_string()?));
        }

        let path_name_value = dwarf.attr_string(&unit, file.path_name())?;
        path.push(path_name_value.to_string()?);

        let dest = Rc::new(path.to_string());

        let file_entries = match res.locations_by_filename.entry(dest) {
          indexmap::map::Entry::Occupied(entry) => entry.into_mut(),
          indexmap::map::Entry::Vacant(entry) => entry.insert(FileEntries {
            language: lang,
            entries: Vec::new(),
          }),
        };

        let loc = LocationEntry {
          addr: code_section_offset + addr,
          pos,
        };

        res.locations.push(loc.clone());
        file_entries.entries.push(loc);

        if row.end_sequence() {
          func_state = FuncState::Start;
        }
      }
    }

    res.locations.sort_by_key(|loc| loc.addr);
    res.locations.dedup_by_key(|loc| loc.addr);

    for file_entries in res.locations_by_filename.values_mut() {
      let entries = &mut file_entries.entries;
      entries.sort_by_key(|loc| loc.pos);
      entries.dedup_by_key(|loc| loc.pos);
    }

    Ok(res)
  }

  fn source_files(&self) -> Result<Vec<SourceFile>, RunError> {
    let mut result = vec![];
    for (filename, entry) in self.locations_by_filename.iter() {
      result.push(SourceFile {
        file: filename.as_ref().clone(),
        language: entry.language,
        lines: entry
          .entries
          .iter()
          .map(|location| {
            vec![
              location.addr,
              location.pos.line as u64,
              location.pos.column as u64,
            ]
          })
          .collect(),
      });
    }
    Ok(result)
  }
}

#[derive(Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct SourceMap {
  files: Vec<String>,
  locations: Vec<Vec<u64>>,
}

#[derive(Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct SourceFile {
  file: String,
  language: u16,
  lines: Vec<Vec<u64>>,
}

#[derive(Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct SourceMapResult {
  #[serde(skip_serializing_if = "Option::is_none")]
  files: Option<Vec<SourceFile>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  error: Option<String>,
}

#[derive(Debug)]
pub enum RunError {
  Resolver(ResolverError),
  Io(std::io::Error),
  Json(serde_json::Error),
  Internal(&'static str),
}

impl fmt::Display for RunError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      RunError::Resolver(err) => write!(f, "resolver error: {}", err),
      RunError::Io(err) => write!(f, "io error: {}", err),
      RunError::Json(err) => write!(f, "json error: {}", err),
      RunError::Internal(msg) => write!(f, "internal error: {}", msg),
    }
  }
}

impl error::Error for RunError {
  fn source(&self) -> Option<&(dyn error::Error + 'static)> {
    match self {
      RunError::Resolver(err) => Some(err),
      RunError::Io(err) => Some(err),
      RunError::Json(err) => Some(err),
      RunError::Internal(_) => None,
    }
  }
}

fn run() -> Result<(), RunError> {
  let mut buffer = Vec::new();
  std::io::stdin()
    .read_to_end(&mut buffer)
    .map_err(RunError::Io)?;
  let slice = EndianSlice::new(buffer.as_slice(), LittleEndian);
  let resolver = Resolver::new(slice).map_err(RunError::Resolver)?;
  let result = SourceMapResult {
    files: Some(resolver.source_files()?),
    error: None,
  };
  serde_json::to_writer(std::io::stdout(), &result).map_err(RunError::Json)?;
  Ok(())
}

fn main() {
  run().unwrap_or_else(|err| {
    let error_doc = SourceMapResult {
      error: Some(err.to_string()),
      files: None,
    };
    serde_json::to_writer(std::io::stdout(), &error_doc).unwrap_or_else(|err| {
      eprintln!("Error: {}", err);
      std::process::exit(2);
    });
    std::process::exit(1);
  });
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  pub fn test_parse() {
    // read a file in
    let src = std::fs::read("./src/count_vowels.rs.wasm").expect("could not read wasm file");
    let resolver = Resolver::new(EndianSlice::new(src.as_slice(), LittleEndian)).unwrap();
    let actual = SourceMapResult {
      files: Some(resolver.source_files().expect("source_map failed")),
      error: None,
    };

    // std::fs::write(
    //   "./src/count_vowels.rs.wasm.json",
    //   serde_json::to_string_pretty(&actual).expect("json failed"),
    // )
    // .unwrap();

    // read count_vowels.rs.wasm.json
    let expected: SourceMapResult = serde_json::from_reader(
      std::fs::File::open("./src/count_vowels.rs.wasm.json").expect("could not read json file"),
    )
    .expect("json failed");
    assert_eq!(actual, expected);
  }
}
