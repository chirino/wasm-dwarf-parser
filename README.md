# WASM Dwarf Parser for Chicory

[![Version](https://img.shields.io/maven-central/v/io.github.chirino/wasm-dwarf-parser?logo=apache-maven&style=flat-square)](https://central.sonatype.com/artifact/io.roastedroot/proxy-wasm-java-host-parent)[![Javadocs](http://javadoc.io/badge/io.github.chirino/wasm-dwarf-parser.svg)](http://javadoc.io/doc/io.github.chirino/wasm-dwarf-parser)

Implements a Debug Parser for WebAssembly modules extracts DWARF debug symbols so that they can be used by the [Chicory WASM Runtime](https://chicory.dev/)

With this module, [Chicory](https://chicory.dev/) can now read **DWARF** debug symbols embedded in a WebAssembly module and use them to enrich Java stack traces with the *original* source-level file names and line numbers that produced an exception.  This is extremely valuable when you are running modules generated from higher-level languages such as **Rust** or **Go** and need to identify exactly where a failure occurred.

Without debug symbols you would normally see a stack trace that only references the Chicory Java interpreter implementation:

```text
com.dylibso.chicory.runtime.TrapException: Trapped on unreachable instruction
	at com.dylibso.chicory.runtime.InterpreterMachine.THROW_UNREACHABLE(InterpreterMachine.java:2212)
	at com.dylibso.chicory.runtime.InterpreterMachine.eval(InterpreterMachine.java:182)
	at com.dylibso.chicory.runtime.InterpreterMachine.call(InterpreterMachine.java:100)
	at com.dylibso.chicory.runtime.InterpreterMachine.CALL(InterpreterMachine.java:1715)
    … more frames trimmed …
```

With the DWARF debug parser enabled the same failure is reported with source context from your WebAssembly module:

```text
com.dylibso.chicory.runtime.TrapException: Trapped on unreachable instruction
	at 0x006721: chicory interpreter.rust_panic_with_hook(library/std/src/sys/pal/wasm/../unsupported/common.rs:28)
	at 0x005cc6: chicory interpreter.{closure#0}(library/std/src/panicking.rs:699)
	at 0x005c00: chicory interpreter.__rust_end_short_backtrace<std::panicking::begin_panic_handler::{closure_env#0}, !>(library/std/src/sys/backtrace.rs:168)
	at 0x00627d: chicory interpreter.begin_panic_handler(library/std/src/panicking.rs:697)
	at 0x007e74: chicory interpreter.panic_nounwind_fmt(library/core/src/panicking.rs:117)
	at 0x007ec8: chicory interpreter.panic_nounwind(library/core/src/panicking.rs:218)
    … more frames trimmed …
```

The same applies when you compile the WASM module with the Chicory compiler – method names now resolve to their **Rust/Go** equivalents and link back to the original line number:

```text
com.dylibso.chicory.runtime.TrapException: Trapped on unreachable instruction
	at com.dylibso.chicory.$gen.CompiledMachineShaded.throwTrapException(Shaded.java:195)
	at com.dylibso.chicory.$gen.CompiledMachineFuncGroup_0.rust_panic_with_hook(library/std/src/sys/pal/wasm/../unsupported/common.rs:28)
	at com.dylibso.chicory.$gen.CompiledMachineFuncGroup_0.{closure#0}(library/std/src/panicking.rs:699)
	at com.dylibso.chicory.$gen.CompiledMachineFuncGroup_0.__rust_end_short_backtrace<std::panicking::begin_panic_handler::{closure_env#0}, !>(library/std/src/sys/backtrace.rs:168)
	at com.dylibso.chicory.$gen.CompiledMachineFuncGroup_0.begin_panic_handler(library/std/src/panicking.rs:697)
	at com.dylibso.chicory.$gen.CompiledMachineFuncGroup_0.panic_nounwind_fmt(library/core/src/panicking.rs:117)
	at com.dylibso.chicory.$gen.CompiledMachineFuncGroup_0.panic_nounwind(library/core/src/panicking.rs:218)
    … more frames trimmed …

```

# Getting Started

## 1. Add the Maven dependency

```xml
<dependency>
    <groupId>io.github.chirino</groupId>
    <artifactId>wasm-dwarf-parser</artifactId>
    <version>${wasm-dwarf-parser.version}</version>
</dependency>
```
## 2. Enable the parser

### Interpreter

Usage with the interpreter:

```java title="Interpreter setup"
import com.dylibso.chicory.dwarf.rust.DebugParser;
import com.dylibso.chicory.runtime.Instance;
import com.dylibso.chicory.wasm.WasmModule;

var module = Parser.parse(new File("your.wasm"));
Instance instance = Instance.builder(module)
        .withDebugParser(DebugParser::parse)
        .build();
```

### Runtime Compiler

Usage with the runtime compiler:

```java title="AOT compiler setup"
import io.github.chirino.wasm.dwarf.rust.DebugParser;
import com.dylibso.chicory.compiler.MachineFactoryCompiler;
import com.dylibso.chicory.runtime.Instance;
import com.dylibso.chicory.wasm.WasmModule;

var module = Parser.parse(new File("your.wasm"));
Instance instance = Instance.builder(module)
        .withMachineFactory(
                MachineFactoryCompiler.builder(module)
                        .withDebugParser(DebugParser::parse)
                        .compile())
        .build();
```

### Build Time Compiler

To enable the Debug parser in the build time compiler, add
the `wasm-dwarf-parser` maven module as a dependency of
the `chicory-compiler-maven-plugin`.

```xml
  <build>
    <plugins>
      <plugin>
        <groupId>com.dylibso.chicory</groupId>
        <artifactId>chicory-compiler-maven-plugin</artifactId>
        <dependencies>
          <dependency>
            <groupId>io.github.chirino</groupId>
            <artifactId>wasm-dwarf-parser</artifactId>
            <version>${wasm-dwarf-parser.version}</version>
          </dependency>
        </dependencies>
        ...
      </plugin>
    </plugins>
  </build>
```

# Limitations

* WASM has NOT standardized on Dwarf for debug symbols.  This may not work for all WASM modules.
* The WebAssembly module must be compiled *with* debug info.
* Performance impact is negligible at runtime, but there is a one-off cost during module loading while the DWARF sections are parsed.

# Feedback

We would love to hear how this feature works for you and which languages/tool-chains you need supported next.  Please file issues or start a discussion on GitHub.

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
