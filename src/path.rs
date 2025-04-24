// Copyright 2019 The Chromium Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use std::borrow::Cow;

pub struct Path<'a>(Cow<'a, str>);

impl<'a> Path<'a> {
  pub fn new(s: Cow<'a, str>) -> Self {
    Path(Cow::from(s.replace("\\\\", "/").replace("\\", "/")))
  }

  pub fn push(&mut self, p2: Cow<'a, str>) {
    if p2.starts_with("/") || p2.contains("://") {
      self.0 = p2;
    } else {
      let p1 = self.0.to_mut();
      if !p1.ends_with('/') {
        p1.push('/');
      }
      p1.push_str(&p2);
    }
  }

  pub fn to_string(&self) -> String {
    (&self.0).to_string()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  pub fn test_path_unix() {
    let mut path = Path::new(Cow::from("/"));
    path.push(Cow::from("etc"));
    path.push(Cow::from("passwd"));
    assert_eq!(path.to_string(), "/etc/passwd");

    let mut path = Path::new(Cow::from("/etc"));
    path.push(Cow::from("passwd"));
    path.push(Cow::from("/etc/hosts"));
    assert_eq!(path.to_string(), "/etc/hosts");
  }

  #[test]
  pub fn test_path_windows() {
    let mut path = Path::new(Cow::from("C:\\"));
    path.push(Cow::from("Windows"));
    path.push(Cow::from("System32"));
    assert_eq!(path.to_string(), "C:/Windows/System32");

    let mut path = Path::new(Cow::from("\\\\"));
    path.push(Cow::from("Server"));
    path.push(Cow::from("Share"));
    assert_eq!(path.to_string(), "/Server/Share");
  }

  #[test]
  pub fn test_path_rustc() {
    let path = Path::new(Cow::from("/rustc/folder/file.rs"));
    assert_eq!(path.to_string(), "/rustc/folder/file.rs");
  }
}
