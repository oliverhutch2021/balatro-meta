-- Facts about the Lua runtime this runs on. Under `make rng-game` that is the
-- game's own lua51.dll (LuaJIT 2.0.5 with the Windows C runtime). Our own
-- script: it only exercises the runtime and prints what it finds.

local fmt = string.format
local function num(x) return fmt("%.17g", x) end

print("version", (jit and jit.version) or _VERSION, (jit and jit.os) or "?", (jit and jit.arch) or "?")

-- How %.13f, %.2f and %.0f round exact halves (2^-14 = 0.00006103515625, ...).
for _, x in ipairs({ 2^-14, 3 * 2^-14, 5 * 2^-14, 0.125, 0.375, 0.5, 1.5, 2.5 }) do
  print("format", num(x), fmt("%.13f", x), fmt("%.2f", x), fmt("%.0f", x))
end

-- The game's 13-place round trip on values that are not ties.
for _, x in ipairs({ 0.1, 1 / 3, 0.99999999999995, 0.12345678901234567 }) do
  print("round13", num(x), fmt("%.13f", x), num(tonumber(fmt("%.13f", x))))
end

-- Non-finite values through the same round trip.
local nan, inf = 0 / 0, 1 / 0
print("nan", fmt("%.13f", nan), tostring(tonumber(fmt("%.13f", nan))), fmt("%.17g", nan))
print("inf", fmt("%.13f", inf), tostring(tonumber(fmt("%.13f", inf))), fmt("%.17g", -inf))

-- LuaJIT's generator right after seeding.
for _, d in ipairs({ 0, 0.5, 0.123456789, 1e-300 }) do
  math.randomseed(d)
  print("random", num(d), num(math.random()), num(math.random()), num(math.random(52)))
end

-- Iteration order of a 12-entry table keyed by the poker-hand names, built by
-- one constructor in the game's listing order (Flush Five first, High Card
-- last), each value a table. Orbital Tag, To Do List and The Ox walk such a
-- table with pairs().
local hands = {
  ["Flush Five"] = {}, ["Flush House"] = {}, ["Five of a Kind"] = {}, ["Straight Flush"] = {},
  ["Four of a Kind"] = {}, ["Full House"] = {}, ["Flush"] = {}, ["Straight"] = {},
  ["Three of a Kind"] = {}, ["Two Pair"] = {}, ["Pair"] = {}, ["High Card"] = {},
}
local order = {}
for k in pairs(hands) do order[#order + 1] = k end
print("hands_pairs_order", table.concat(order, "|"))
