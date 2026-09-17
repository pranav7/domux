-- With one window every direction is an edge, so every move hands focus back to domux.
T.claimed()
for _, dir in ipairs({ "left", "down", "up", "right" }) do
  T.hand_back(dir)
end
assert(require("domux").state.last.ok, vim.inspect(require("domux").state.last))
