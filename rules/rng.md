# Random numbers

How Balatro turns a seed into every random event, in our own words. The Rust
port is `crates/bm-rng`; it is checked value for value against LuaJIT
(`make rng-fixtures`, `make rng-stress`).

## The seed

- A seed is 1 to 8 characters from `1-9` and `A-Z`. A typed `0` becomes `O`.
  There are about 2.3 trillion seeds of up to 8 characters.
- The seed string is hashed once into a number in [0, 1) (`pseudohash`): start
  from 1 and walk the string from its last character to its first; at position
  `i` with byte `b`, the running value becomes
  `((1.1239285023 / n) * b * pi + pi * i) mod 1`.
- A few strings make the running value collapse to 0, after which it becomes
  NaN. LuaJIT prints and re-reads NaN without error, so the game carries
  the NaN on and we do the same (example: key `erratic` with seed `7LB2WVPK`).

## Named streams

- Every kind of random event draws from its own named stream, such as the deck
  shuffle, a shop slot's rarity, or a Lucky card's roll. Drawing from one stream
  never moves another, which is what lets two strategies share a world until
  their choices differ.
- A stream starts at the hash of `key .. seed`. Each draw first steps it:
  `v = |round13((2.134453429141 + v * 1.72431234) mod 1)|`, where `round13`
  prints the number to 13 decimal places and reads it back.
- LuaJIT prints exact halves away from zero (2^-14 prints as
  `0.0000610351563`), unlike C and Rust, which round them to even. Our
  `round13` does the same arithmetic exactly with integers.
- The draw's seed value is `(v + hash(seed)) / 2`. That value reseeds LuaJIT's
  generator (`math.randomseed`), and the event takes its numbers from it:
  one number for a roll, several for a shuffle or a pick from a list.
- Because every stream shares `hash(seed)`, outputs of different streams are
  not fully independent. The fair-play RNG-swap check exists for this reason.

## LuaJIT's generator

- LuaJIT 2.1's `math.random` is a combined Tausworthe generator (TW223) with
  four 64-bit words. `math.randomseed(d)` fills them from `d` by repeating
  `d = d * pi + e`, then discards ten outputs.
- `math.random()` gives a double in [0, 1); `math.random(m)` gives
  `floor(r * m) + 1`; `math.random(a, b)` gives `floor(r * (b - a + 1)) + a`.

## The game's runtime

- Balatro 1.0.1o ships LuaJIT **2.0.5** (`lua51.dll`), linked to the Windows
  Universal C runtime; our cloud fixtures come from LuaJIT 2.1.
- `math.random` is the same TW223 generator in both. `make rng-check` runs our
  reference script on the game's own `lua51.dll` to confirm it value for value.
- In 2.0.5, `string.format` hands numbers to the C runtime's `sprintf`. The
  Universal CRT prints the exact binary value correctly rounded, with exact
  halves rounded to even (Windows 10 2004 and later). LuaJIT 2.1 has its own
  formatter that rounds exact halves away from zero, so the reference script
  corrects ties when it is not running on the game's runtime, and our
  `round13` rounds halves to even. Exact ties at the 13th place are the values
  k / 2^14 with k odd, so they are rare in practice, but they are handled.
- `tonumber` uses LuaJIT's own exact parser in both versions.
- NaN: a string's hash collapses to NaN only in freak cases (none in 60 million
  random key+seed strings). How the game's runtime prints and re-reads NaN is
  recorded by the probe in `results/rng/game_runtime.txt`.

## Shuffles and picks

- A shuffle walks the list from the end: for `i = n` down to 2, swap item `i`
  with item `math.random(i)`, all from one seeding.
- A pick takes `math.random(#list)` from one seeding.
- Both depend on the list's order beforehand. The game puts lists into a
  canonical order first; the engine must reproduce that order (phase 2 notes).

## Sources

- [Balatro's pseudorandom numbers](https://rennsax.com/posts/balatro-pseudorandom/):
  the hash, the stream step and LuaJIT's generator.
- [Balatro's least erratic decks](https://blog.danlew.net/2025/11/03/balatros-least-erratic-decks/):
  the seed alphabet, the seed count and the NaN seeds.
- LuaJIT 2.1 (`math.random`, `math.randomseed`, `string.format`), MIT licence.
