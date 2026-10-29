"""Run a Lua script on the game's own LuaJIT (lua51.dll). Runs under *Windows* Python.

    python.exe win_lua_runner.py <balatro_dir> <script.lua> <out_file> [script args...]

Loads <balatro_dir>\\lua51.dll (LuaJIT 2.0.5 with the Windows C runtime, exactly what the
game runs on), runs the script with `arg` set to the extra arguments, and writes everything
the script prints to <out_file>, one line per print. Only the Python standard library is
used. The game's files are loaded in place, never copied. `scripts/rng_game.py` calls this
from Ubuntu through WSL's Windows interop.
"""
from __future__ import annotations

import ctypes
import os
import sys


def lua_string(s: str) -> str:
    """A Lua long-bracket literal for s (paths and simple arguments)."""
    eq = "="
    while f"]{eq}]" in s:
        eq += "="
    return f"[{eq}[{s}]{eq}]"


def main() -> int:
    if len(sys.argv) < 4:
        print(__doc__, file=sys.stderr)
        return 2
    bdir, script, out = sys.argv[1], sys.argv[2], sys.argv[3]
    args = sys.argv[4:]
    if hasattr(os, "add_dll_directory"):
        os.add_dll_directory(bdir)  # lua51.dll's own dependencies sit next to it
    # BM_LUA_LIB lets the tests drive the same code with another LuaJIT build (e.g. on Linux).
    lua = ctypes.CDLL(os.environ.get("BM_LUA_LIB") or os.path.join(bdir, "lua51.dll"))
    lua.luaL_newstate.restype = ctypes.c_void_p
    lua.luaL_openlibs.argtypes = [ctypes.c_void_p]
    lua.luaL_loadbuffer.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_size_t, ctypes.c_char_p]
    lua.luaL_loadbuffer.restype = ctypes.c_int
    lua.lua_pcall.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int]
    lua.lua_pcall.restype = ctypes.c_int
    lua.lua_tolstring.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.POINTER(ctypes.c_size_t)]
    lua.lua_tolstring.restype = ctypes.c_void_p
    lua.lua_settop.argtypes = [ctypes.c_void_p, ctypes.c_int]
    lua.lua_close.argtypes = [ctypes.c_void_p]

    L = lua.luaL_newstate()
    lua.luaL_openlibs(L)

    def run(code: str, name: str) -> None:
        data = code.encode("utf-8")
        rc = lua.luaL_loadbuffer(L, data, len(data), ("=" + name).encode())
        if rc == 0:
            rc = lua.lua_pcall(L, 0, 0, 0)
        if rc != 0:
            n = ctypes.c_size_t(0)
            p = lua.lua_tolstring(L, -1, ctypes.byref(n))
            msg = ctypes.string_at(p, n.value).decode("utf-8", "replace") if p else "(no message)"
            lua.lua_settop(L, 0)
            raise SystemExit(f"Lua error in {name}: {msg}")

    # Prelude: arguments, and print() redirected to the output file (streamed, so big outputs
    # never sit in LuaJIT 2.0's limited memory).
    arg_list = ", ".join(lua_string(a) for a in args)
    run(f"arg = {{ {arg_list} }}\n"
        f"__BM_OUT = assert(io.open({lua_string(out)}, 'wb'))\n"
        "print = function(...)\n"
        "  local n = select('#', ...)\n"
        "  for i = 1, n do\n"
        "    if i > 1 then __BM_OUT:write('\\t') end\n"
        "    __BM_OUT:write(tostring((select(i, ...))))\n"
        "  end\n"
        "  __BM_OUT:write('\\n')\n"
        "end\n", "prelude")
    with open(script, "r", encoding="utf-8") as fh:
        source = fh.read()
    try:
        run(source, os.path.basename(script))
    finally:
        run("__BM_OUT:close()", "close")
        lua.lua_close(L)
    return 0


if __name__ == "__main__":
    sys.exit(main())
