// Copyright 2019 The Chromium Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

mod path;
mod wasm;

use fallible_iterator::FallibleIterator;
use gimli::{ColumnType, Dwarf, EndianSlice, LittleEndian, Reader, ReaderOffset};
use indexmap::IndexMap;
use path::Path;
use serde::{Deserialize, Serialize};
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
  filename: Rc<String>,
  addr: u64,
  pos: Pos,
}

pub struct FileEntries {
  filename: Rc<String>,
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

      let is_rust = {
        let mut entries = unit.entries();
        entries.next_entry()?;
        match entries
          .current()
          .unwrap()
          .attr_value(gimli::DW_AT_language)?
        {
          Some(gimli::AttributeValue::Language(gimli::constants::DW_LANG_Rust)) => true,
          _ => false,
        }
      };

      let unit_dir = Path::new(
        unit
          .comp_dir
          .as_ref()
          .map(|comp_dir| comp_dir.to_string())
          .transpose()?
          .unwrap_or_default(),
      )
      .map_err(ResolverError::InvalidPath)?;

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

        let mut path = unit_dir.borrow();

        let dir_value;
        if let Some(dir) = file.directory(header) {
          dir_value = dwarf.attr_string(&unit, dir)?;
          path.push(dir_value.to_string()?);
        }

        let path_name_value = dwarf.attr_string(&unit, file.path_name())?;
        path.push(path_name_value.to_string()?);

        let dest = Rc::new(path.to_uri());

        let file_entries = match res.locations_by_filename.entry(dest) {
          indexmap::map::Entry::Occupied(entry) => entry.into_mut(),
          indexmap::map::Entry::Vacant(entry) => {
            let filename = entry.key().clone();
            entry.insert(FileEntries {
              filename,
              entries: Vec::new(),
            })
          }
        };

        let loc = LocationEntry {
          filename: file_entries.filename.clone(),
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

  fn source_map(&self) -> Result<SourceMap, RunError> {
    let mut result = SourceMap {
      files: vec![],
      locations: vec![],
    };

    let mut file_indexes: HashMap<Rc<String>, usize> = HashMap::new();
    for (filename, _) in self.locations_by_filename.iter() {
      file_indexes.insert(filename.clone(), result.files.len());
      result.files.push(filename.as_ref().clone());
    }
    for location in &self.locations {
      let file_index = file_indexes
        .get(&location.filename)
        .ok_or(RunError::Internal("fileIndex did not contain file"))?;
      result.locations.push(vec![
        location.addr,
        *file_index as u64,
        location.pos.line as u64,
        location.pos.column as u64,
      ])
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
pub struct ErrorDoc {
  error: String,
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
  let source_map = resolver.source_map()?;
  serde_json::to_writer(std::io::stdout(), &source_map).map_err(RunError::Json)?;
  Ok(())
}

fn main() {
  run().unwrap_or_else(|err| {
    let error_doc = ErrorDoc {
      error: err.to_string(),
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
    let source_map = resolver.source_map().expect("source_map failed");

    // read count_vowels.rs.wasm.json
    let expected: SourceMap = serde_json::from_reader(
      std::fs::File::open("./src/count_vowels.rs.wasm.json").expect("could not read json file"),
    )
    .expect("json failed");
    assert_eq!(source_map, expected);
  }
}
