-- The plugin claims the keys at startup and releases them at exit; run.lua checks both requests.
T.claimed()
local claim = require("domux").state.claim
assert(claim.ok, vim.inspect(claim))
assert(#T.messages() == 0, vim.inspect(T.messages()))
