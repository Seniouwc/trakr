-- Song report: prints the tempo, track list and how many notes each pattern holds.
local s = trakr.song()
print(s.name .. " - " .. s.bpm .. " BPM, " .. s.lpb .. " lines per beat, " .. s.tpl .. " ticks per line")
for t = 0, s.tracks - 1 do print("track " .. t .. ": " .. trakr.track_name(t)) end
for p = 0, s.patterns - 1 do
  local notes = 0
  for t = 0, s.tracks - 1 do
    for l = 0, trakr.pattern_lines(p) - 1 do
      if trakr.get_note(p, t, l, 0).note < 120 then notes = notes + 1 end
    end
  end
  print(string.format("pattern %02X: %d lines, %d notes in first columns", p, trakr.pattern_lines(p), notes))
end
