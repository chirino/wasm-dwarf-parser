// Copyright 2019 The Chromium Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

mod apperror;
mod path;
mod wasm;

use apperror::Error;
use fallible_iterator::FallibleIterator;
use gimli::{constants, ColumnType, Dwarf, EndianSlice, LittleEndian, Reader, ReaderOffset};
use indexmap::IndexMap;
use path::Path;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::convert::TryInto;
use std::io::Read;
use wasm::{parse_sections, SectionKind};

pub struct Pos {
  line: u32,
  column: u32,
}

enum FuncState {
  Start,
  Ignored,
  Normal,
}

#[derive(Default, Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct SourceFile {
  file: String,
  language: u16,
  lines: Vec<Vec<u64>>,
}

#[derive(Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct SourceUnit {
  name: String,
  directory: String,
  files: Vec<SourceFile>,
}

#[derive(Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct SourceResult {
  #[serde(skip_serializing_if = "Option::is_none")]
  units: Option<Vec<SourceUnit>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  error: Option<String>,
}

pub fn extract_soruce_info<R: Reader + Clone + Default>(src: R) -> Result<SourceResult, Error> {
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
    .ok_or_else(|| Error::MissingCodeSection)?
    .into_u64()
    .try_into()
    .unwrap();

  let dwarf = Dwarf::load::<_, _, Error>(
    |id| Ok(sections.get(id.name()).cloned().unwrap_or_default()),
    |_| Ok(Default::default()),
  )?;

  let mut res = vec![];

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

    let mut source_unit = SourceUnit::default();
    let mut locations_by_filename: IndexMap<String, SourceFile> = IndexMap::new();

    source_unit.name = unit
      .name
      .as_ref()
      .map(|x| x.to_string())
      .transpose()?
      .unwrap_or_default()
      .to_string();

    source_unit.directory = unit
      .comp_dir
      .as_ref()
      .map(|x| x.to_string())
      .transpose()?
      .unwrap_or_default()
      .to_string();

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
      let mut path = Path::new(Cow::from("."));

      let dir_value;
      if let Some(dir) = file.directory(header) {
        dir_value = dwarf.attr_string(&unit, dir)?;
        path = Path::new(Cow::from(dir_value.to_string()?));
      }

      let path_name_value = dwarf.attr_string(&unit, file.path_name())?;
      path.push(path_name_value.to_string()?);

      let dest = path.to_string();

      let file_entries = match locations_by_filename.entry(dest.clone()) {
        indexmap::map::Entry::Occupied(entry) => entry.into_mut(),
        indexmap::map::Entry::Vacant(entry) => entry.insert(SourceFile {
          file: dest.to_string(),
          language: lang,
          lines: Vec::new(),
        }),
      };

      file_entries.lines.push(vec![
        code_section_offset + addr,
        pos.line as u64,
        pos.column as u64,
      ]);

      if row.end_sequence() {
        func_state = FuncState::Start;
      }
    }

    for file_entries in locations_by_filename.values_mut() {
      let entries = &mut file_entries.lines;
      entries.sort_by_key(|loc| loc[0]);
      entries.dedup_by_key(|loc| loc[0]);
    }
    source_unit.files = locations_by_filename.values().cloned().collect();
    if !source_unit.files.is_empty() {
      res.push(source_unit);
    }
  }
  Ok(SourceResult {
    units: Some(res),
    error: None,
  })
}

fn run() -> Result<(), Error> {
  let mut buffer = Vec::new();
  std::io::stdin()
    .read_to_end(&mut buffer)
    .map_err(Error::Io)?;
  let slice = EndianSlice::new(buffer.as_slice(), LittleEndian);
  let result = extract_soruce_info(slice)?;
  serde_json::to_writer(std::io::stdout(), &result).map_err(Error::Json)?;
  Ok(())
}

fn main() {
  run().unwrap_or_else(|err| {
    let error_doc = SourceResult {
      error: Some(err.to_string()),
      units: None,
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
    let actual = extract_soruce_info(EndianSlice::new(src.as_slice(), LittleEndian))
      .expect("extract_soruce_info failed");

    // std::fs::write(
    //   "./src/count_vowels.rs.wasm.json",
    //   serde_json::to_string_pretty(&actual).expect("json failed"),
    // )
    // .unwrap();

    // read count_vowels.rs.wasm.json
    let expected: SourceResult = serde_json::from_reader(
      std::fs::File::open("./src/count_vowels.rs.wasm.json").expect("could not read json file"),
    )
    .expect("json failed");
    assert_eq!(actual, expected);
  }
}
