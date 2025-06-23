# WASM Dwarf Parser for Chicory

[![Version](https://img.shields.io/maven-central/v/io.github.chirino/wasm-dwarf-parser?logo=apache-maven&style=flat-square)](https://central.sonatype.com/artifact/io.roastedroot/proxy-wasm-java-host-parent)[![Javadocs](http://javadoc.io/badge/io.github.chirino/wasm-dwarf-parser.svg)](http://javadoc.io/doc/io.github.chirino/wasm-dwarf-parser)

Implements a Debug Parser for WebAssembly modules extracts DWARF debug symbols so that they can be used by the [Chicory WASM Runtime](https://chicory.dev/)


## Building

To rebuild the rust wasm module run:

```sh
./build.sh
```

or run mvn with the `-P build-rust` argument:

```sh
mvn clean install -P build-rust
```

## Attribution

Rust bits forked from https://github.com/bmeurer/wasm-source-map
