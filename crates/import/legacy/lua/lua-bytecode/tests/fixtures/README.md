# Lua bytecode fixtures

These fixtures are small programs written for this repository. Each `.luac`
file under `linear/` or `control_flow/` was compiled from the matching Lua file
under that directory's `src/` folder with the stock Lua 5.1.5 compiler.

The compiler is run from the fixture category directory with a relative source
path. This keeps debug metadata portable and prevents local filesystem paths
from entering the bytecode.
