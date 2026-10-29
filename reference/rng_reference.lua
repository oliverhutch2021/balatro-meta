-- Reference fixtures for the Rust RNG port (crates/bm-rng).
--
-- Run with LuaJIT:  luajit reference/rng_reference.lua > tests/fixtures/rng/reference.tsv
-- or on the game's own runtime (LuaJIT 2.0.5 + the Windows C runtime): `make rng-game`.
-- An optional scale argument multiplies the number of generated cases
-- (`make rng-stress` uses it); scale 1 reproduces the committed fixture.
--
-- LuaJIT's own math.random/math.randomseed are the reference for the
-- generator. The seed hash and named streams below are written from the
-- public description of Balatro's recipe (see rules/rng.md for sources); they
-- are not taken from the game's files.

local fmt = string.format
local SCALE = tonumber(arg and arg[1]) or 1
local function num(x) return fmt("%.17g", x) end

-- Deterministic test inputs (Park-Miller LCG, independent of math.random).
local lcg_state = 20261029
local function lcg()
  lcg_state = (lcg_state * 16807) % 2147483647
  return lcg_state
end

local ALPHABET = "123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ"
local function gen_seed(len)
  local t = {}
  for i = 1, len do
    local k = lcg() % #ALPHABET + 1
    t[i] = ALPHABET:sub(k, k)
  end
  return table.concat(t)
end

-- Rounding to 13 places ------------------------------------------------------

-- The game runs LuaJIT 2.0.5, whose string.format uses the C runtime's sprintf
-- (on Windows the Universal CRT: exact, halves to even). On that runtime the
-- plain round trip below is the reference. LuaJIT 2.1 has its own formatter
-- that rounds exact halves away from zero, so elsewhere ties are corrected.
-- Exact ties at the 13th decimal are exactly the values k / 2^14 with k odd.
local GAME_RUNTIME = (jit ~= nil and jit.version == "LuaJIT 2.0.5")
local function round13(x)
  if not GAME_RUNTIME then
    local t = x * 16384
    if x > 0 and x < 1 and t == math.floor(t) and t % 2 == 1 then
      local twice = t * 1220703125            -- 2 * x * 10^13, an odd integer
      local lo = (twice - 1) / 2
      local n = (lo % 2 == 0) and lo or lo + 1
      return tonumber(fmt("0.%013d", n))
    end
  end
  return tonumber(fmt("%.13f", x))
end

-- The recipe -----------------------------------------------------------------

local function pseudohash(str)
  local n = 1
  for i = #str, 1, -1 do
    n = ((1.1239285023 / n) * string.byte(str, i) * math.pi + math.pi * i) % 1
  end
  return n
end

local function new_streams(seed)
  return { seed = seed, hashed = pseudohash(seed), state = {} }
end

local function pseudoseed(st, key)
  if st.state[key] == nil then
    st.state[key] = pseudohash(key .. st.seed)
  end
  st.state[key] = math.abs(round13((2.134453429141 + st.state[key] * 1.72431234) % 1))
  return (st.state[key] + st.hashed) / 2
end

local function pseudorandom(st, key, lo, hi)
  math.randomseed(pseudoseed(st, key))
  if lo and hi then return math.random(lo, hi) end
  return math.random()
end

local function shuffle(list)
  for i = #list, 2, -1 do
    local j = math.random(i)
    list[i], list[j] = list[j], list[i]
  end
end

-- Inputs ---------------------------------------------------------------------

local doubles = { 0, 0.5, 1, -1, 2, 0.1, 1e-300, -1e-300, 123456.789, math.pi,
                  0.99999999999999989, 1e15, -1e15, 4503599627370496.5 }
for _ = 1, 300 * SCALE do
  local d = lcg() / 2147483647
  doubles[#doubles + 1] = d
  doubles[#doubles + 1] = d * 1000 - 500
end

local seeds = { "TUTORIAL", "DFRU5D52", "7LB2WVPK", "1", "A", "AAAAAAAA", "ZZZZZZZZ",
                "99999999", "OOOOOOOO", "11111111", "ABCDEFGH", "BALATRO", "JOKER" }
for _ = 1, 150 * SCALE do seeds[#seeds + 1] = gen_seed(8) end
for _ = 1, 20 * SCALE do seeds[#seeds + 1] = gen_seed(lcg() % 7 + 1) end

local keys = { "shuffle", "nr1", "nr2", "cdt1", "rarity1sho", "Joker11sho", "edisho1",
               "lucky_mult", "lucky_money", "wheel_of_fortune", "boss", "Tag1", "stdset1",
               "front", "erratic", "misprint", "glass", "hook", "cavendish",
               "gros_michel", "Voucher1", "soul_Tarot1", "", "x" }

-- Output ---------------------------------------------------------------------

print("META\t" .. ((jit and jit.version) or _VERSION) .. "\t" .. (GAME_RUNTIME and "game-runtime" or "ties-corrected"))

-- LuaJIT generator: plain draws, draws in [1, m], draws in [a, b].
for _, d in ipairs(doubles) do
  math.randomseed(d)
  local out = {}
  for i = 1, 12 do out[i] = num(math.random()) end
  print("RAND\t" .. num(d) .. "\t" .. table.concat(out, ","))
  for _, m in ipairs({ 1, 2, 7, 52, 1000 }) do
    math.randomseed(d)
    out = {}
    for i = 1, 8 do out[i] = num(math.random(m)) end
    print("RANDN\t" .. num(d) .. "\t" .. m .. "\t" .. table.concat(out, ","))
  end
  math.randomseed(d)
  out = {}
  for i = 1, 8 do out[i] = num(math.random(3, 9)) end
  print("RANDAB\t" .. num(d) .. "\t3\t9\t" .. table.concat(out, ","))
end

-- Hashes of seeds and key..seed strings.
for _, s in ipairs(seeds) do
  print("HASH\t" .. s .. "\t" .. num(pseudohash(s)))
  for _, k in ipairs(keys) do
    print("HASH\t" .. k .. s .. "\t" .. num(pseudohash(k .. s)))
  end
end

-- Rounding to 13 places via string.format, including exact ties and values
-- that round up to 1.
local r13 = { 0, 0.5, 2^-14, 3 * 2^-14, 2^-20, 0.99999999999995, 0.999999999999949,
              0.9999999999999, 0.12345678901234567, 1e-14, 5e-14, 0.00006103515625 }
for _ = 1, 2000 * SCALE do r13[#r13 + 1] = lcg() / 2147483647 end
for _ = 1, 500 * SCALE do r13[#r13 + 1] = (lcg() % 16384) / 16384 end
for _, x in ipairs(r13) do
  print("R13\t" .. num(x) .. "\t" .. fmt("%.13f", x) .. "\t" .. num(round13(x)))
end

-- Named streams: interleaved pseudoseed calls, three rounds over every key.
for _, s in ipairs(seeds) do
  local st = new_streams(s)
  local out = {}
  for _ = 1, 3 do
    for _, k in ipairs(keys) do out[#out + 1] = num(pseudoseed(st, k)) end
  end
  print("SEQ\t" .. s .. "\t" .. table.concat(out, ","))
end

-- pseudorandom draws: a double, then an integer in [1, 100], per key.
for _, s in ipairs(seeds) do
  local st = new_streams(s)
  local out = {}
  for _, k in ipairs(keys) do
    out[#out + 1] = num(pseudorandom(st, k))
    out[#out + 1] = num(pseudorandom(st, k, 1, 100))
  end
  print("PR\t" .. s .. "\t" .. table.concat(out, ","))
end

-- Shuffles of 1..52 seeded from a stream.
for _, s in ipairs(seeds) do
  local st = new_streams(s)
  for _, k in ipairs({ "nr1", "shuffle" }) do
    math.randomseed(pseudoseed(st, k))
    local list = {}
    for i = 1, 52 do list[i] = i end
    shuffle(list)
    print("SHUF\t" .. s .. "\t" .. k .. "\t" .. table.concat(list, ","))
  end
end
