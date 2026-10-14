-- Humanize: randomise the volume of every note in the selection (or the whole cursor track).
local AMOUNT = 24 -- how far volumes may wander, out of 128

local p = trakr.ctx.pattern
local sel = trakr.ctx.selection
local t0, t1, l0, l1
if sel then
  t0, t1, l0, l1 = sel.track0, sel.track1, sel.line0, sel.line1
else
  t0, t1, l0, l1 = trakr.ctx.track, trakr.ctx.track, 0, trakr.pattern_lines(p) - 1
end
local n = 0
for t = t0, t1 do
  for l = l0, l1 do
    for c = 0, 11 do
      local cell = trakr.get_note(p, t, l, c)
      if cell.note < 120 then
        local v = (cell.volume or 128) + math.random(-AMOUNT, AMOUNT)
        trakr.set_note(p, t, l, c, { volume = math.max(0, math.min(128, v)) })
        n = n + 1
      end
    end
  end
end
trakr.status("Humanized " .. n .. " notes")
