# Building the Rust host on Windows (no VS / no MinGW-gcc)

Toolchain: `stable-x86_64-pc-windows-gnu` + LLVM tooling, all via winget:

```
winget install -e --id LLVM.LLVM
winget install -e --id MartinStorsjo.LLVM-MinGW.MSVCRT
rustup target add x86_64-pc-windows-gnullvm   # optional; gnu is the default
```

The gnu target's linker is `x86_64-w64-mingw32-clang` from the llvm-mingw
MSVCRT package (see `rust/.cargo/config.toml`). Two env requirements:

```
PATH=<llvm-mingw-msvcrt>/bin:<cargo bin>:<LLVM bin>:...
LIBRARY_PATH=<repo>/rust/.libalias
```

`rust/.libalias` holds `libgcc.a` / `libgcc_eh.a` aliases onto compiler-rt
builtins + libunwind — rustc's `-lgcc`/`-lgcc_eh` have no MinGW-gcc to come
from, and `-L` rustflags do not reach the linker reliably, so LIBRARY_PATH
is the mechanism. MSVCRT (not UCRT) is deliberate: this machine has no
`api-ms-win-crt-*` forwarders, but `msvcrt.dll` is present.

Then:

```
cd rust && cargo build --target x86_64-pc-windows-gnu
# -> target/x86_64-pc-windows-gnu/debug/ultima3d-server.exe
```

The server resolves `ultima3d/blender_worker.py` and `output/` from the
current working directory, exactly like `python -m ultima3d.server`, so run
it (or point the MCP client) at the repo root.
