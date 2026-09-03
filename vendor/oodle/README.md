# Oodle SDK setup

Oodle is a family of proprietary compression libraries from Rad Game Tools.
Azoth uses Oodle Data for package compression. Oodle Network and Oodle Texture
are fetched alongside it so their integrations do not need a second setup pass.

The libraries are licensed, not redistributable, and never committed to this
repository. Everything under `vendor/oodle/` except this file is ignored by git
and by Lore.

## Where the SDK comes from

Epic ships the Oodle SDKs with Unreal Engine source. Link your Epic account to
GitHub, clone the Unreal Engine repository, and run its `Setup` script so the
SDK files land in the checkout. Your Unreal license covers using those libraries
in your own projects.

Azoth reads the Unreal checkout without changing it.

## Fetch the SDKs

```sh
cargo run -p oodle-fetch -- --unreal-root /path/to/UnrealEngine
```

The tool reads `Engine/Build/Commit.gitdeps.xml`, downloads the packs that hold
the Oodle libraries for your platform, verifies each file against the SHA-1 in
the manifest, and copies the headers out of the checkout.

Useful flags:

- `--platform win64|linux|linux-arm64|mac|all` selects targets. Repeat the flag
  for several. The default is the platform you are running on.
- `--sdk-version 2.9.16` picks the SDK version. This is the default.
- `--destination <dir>` overrides the output root. The default is
  `<azoth-data-home>/oodle/<version>`, which is `~/.azoth/oodle/2.9.16` unless
  `AZOTH_HOME` says otherwise.
- `--dry-run` lists the files, their sizes, and the pack count without
  downloading or writing anything.

## Output layout

Each product gets one flat directory per kind, so a single directory can be
handed to the build:

```text
<destination>/
  data/include/       oodle2.h, oodle2base.h
  data/lib/           the Oodle Data libraries
  network/include/    oodle2net.h, oodle2base.h
  network/lib/        the Oodle Network libraries
  texture/include/    oodle2tex.h, oodle2texrt.h, oodle2base.h
  texture/lib/        the Oodle Texture link libraries
  texture/bin/        the Oodle Texture redistributable shared libraries
```

## Point the build at Oodle Data

Set `OODLE_LIB_DIR` to the Data library directory.

```sh
export OODLE_LIB_DIR="$HOME/.azoth/oodle/2.9.16/data/lib"
```

```powershell
$env:OODLE_LIB_DIR = "$env:USERPROFILE\.azoth\oodle\2.9.16\data\lib"
```

The tool prints the exact value to set when it finishes.

## Verify

```sh
cargo check --locked --features oodle
cargo check --locked -p az-pak --features oodle
```

The build script looks for the library your target needs:
`oo2core_win64.lib` on Windows, `liboo2corelinux64.a` or `.so` on Linux x86_64,
`liboo2corelinuxarm64.a` or `.so` on Linux aarch64, and `liboo2coremac64.a` on
macOS.

Without the `oodle` feature, Azoth reports Oodle packages as unsupported and
other package formats still work. Oodle is a Cargo build dependency, not an
Azoth runtime gem, so only targets built with an `oodle` feature need the
library.
