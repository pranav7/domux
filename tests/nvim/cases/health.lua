T.claimed()
T.hand_back("left")
local report = T.health()
for _, want in ipairs({ "In domux pane p_test", "version 9.9.9", "passes C-h", "focus.left" }) do
  assert(report:find(want, 1, true), want .. " is not in the report:\n" .. report)
end
