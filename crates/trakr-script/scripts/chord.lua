-- Chord stack: turn each note in the cursor track's first column into a minor triad across
-- columns 1-3 (change INTERVALS for other chords).
local INTERVALS = { 0, 3, 7 }

local p, t = trakr.ctx.pattern, trakr.ctx.track
local n = 0
for l = 0, trakr.pattern_lines(p) - 1 do
  local root = trakr.get_note(p, t, l, 0)
  if root.note < 120 then
    for i, semis in ipairs(INTERVALS) do
      trakr.set_note(p, t, l, i - 1, {
        note = math.min(119, root.note + semis),
        instrument = root.instrument,
        volume = root.volume,
      })
    end
    n = n + 1
  end
end
trakr.status("Stacked " .. n .. " chords (the track needs " .. #INTERVALS .. " note columns)")
