-- Euclidean rhythm: spread N hits over the pattern on the cursor track.
-- Edit HITS and NOTE below, then run. Uses the selected instrument.
local HITS = 5
local NOTE = "C-4"

local p, t = trakr.ctx.pattern, trakr.ctx.track
local steps = trakr.pattern_lines(p)
local lpb = trakr.song().lpb
-- one bar worth of steps is repeated across the pattern
local bar = math.min(steps, lpb * 4)
for l = 0, steps - 1 do trakr.clear_note(p, t, l, 0) end
local bucket = 0
for i = 0, bar - 1 do
  bucket = bucket + HITS
  if bucket >= bar then
    bucket = bucket - bar
    for l = i, steps - 1, bar do
      trakr.set_note(p, t, l, 0, { note = NOTE, instrument = trakr.ctx.instrument })
    end
  end
end
trakr.status("Euclidean rhythm: " .. HITS .. " hits per " .. bar .. " lines")
