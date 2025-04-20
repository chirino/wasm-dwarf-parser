// Copyright 2019 The Chromium Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use fallible_iterator::FallibleIterator;
use gimli::{Reader, ReaderOffset};
use std::convert::TryInto;
use std::num::NonZeroU8;
use std::{error, fmt};

#[derive(Debug)]
pub enum ResolverError {
  InvalidMagic,
  UnsupportedVersion(u32),
  MissingCodeSection,
  Reader(gimli::Error),
}

impl fmt::Display for ResolverError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      ResolverError::InvalidMagic => write!(f, "WebAssembly magic mismatch."),
      ResolverError::UnsupportedVersion(v) => {
        write!(f, "Unsupported WebAssembly version {}.", v)
      }
      ResolverError::MissingCodeSection => write!(f, "Missing code section."),
      ResolverError::Reader(err) => write!(f, "{}", err),
    }
  }
}

impl error::Error for ResolverError {
  fn source(&self) -> Option<&(dyn error::Error + 'static)> {
    match self {
      ResolverError::Reader(err) => Some(err),
      _ => None,
    }
  }
}

impl From<gimli::Error> for ResolverError {
  fn from(err: gimli::Error) -> Self {
    ResolverError::Reader(err)
  }
}

#[derive(Debug)]
pub enum SectionKind {
  Custom { name: String },
  Standard { id: NonZeroU8 },
}

#[derive(Debug)]
pub struct Section<R> {
  pub kind: SectionKind,
  pub payload: R,
}

pub fn parse_sections<R: Reader>(
  mut reader: R,
) -> Result<impl FallibleIterator<Item = Section<R>, Error = gimli::Error>, ResolverError> {
  struct Iterator<R> {
    reader: R,
  }

  impl<R: Reader> FallibleIterator for Iterator<R> {
    type Item = Section<R>;
    type Error = gimli::Error;

    fn next(&mut self) -> Result<Option<Section<R>>, gimli::Error> {
      if self.reader.is_empty() {
        return Ok(None);
      }

      let id = self
        .reader
        .read_uleb128()?
        .try_into()
        .map_err(|_| gimli::Error::BadUnsignedLeb128)?;

      let payload_len = ReaderOffset::from_u64(self.reader.read_uleb128()?)?;
      let mut payload_reader = self.reader.split(payload_len)?;

      let kind = match NonZeroU8::new(id) {
        None => {
          let name_len = ReaderOffset::from_u64(payload_reader.read_uleb128()?)?;
          let name_reader = payload_reader.split(name_len)?;
          SectionKind::Custom {
            name: name_reader.to_string()?.into_owned(),
          }
        }
        Some(id) => SectionKind::Standard { id },
      };

      Ok(Some(Section {
        kind,
        payload: payload_reader,
      }))
    }
  }

  if reader.read_u8_array::<[u8; 4]>()? != *b"\0asm" {
    return Err(ResolverError::InvalidMagic);
  }

  let version = reader.read_u32()?;
  if version != 1 {
    return Err(ResolverError::UnsupportedVersion(version));
  }

  Ok(Iterator { reader })
}
