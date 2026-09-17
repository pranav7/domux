-- Loaded with --cmd before the plugin in every case of tests/nvim/run.lua. It records what the
-- plugin tells the reader and gives a case its few helpers.

_G.notifications = {}
vim.notify = function(message, level)
  table.insert(_G.notifications, { message = message, level = level })
end

_G.T = {}

-- Runs a case file. Neovim exits 0 when the case returns and 1, with the error on stderr, when
-- it raises one. Both run VimLeavePre, so the plugin sends its release either way.
function T.run(path)
  local ok, err = pcall(dofile, path)
  if ok then
    vim.cmd("qall!")
  else
    io.stderr:write(tostring(err) .. "\n")
    vim.cmd("cquit 1")
  end
end

-- Waits for the claim the plugin made at startup to be answered.
function T.claimed()
  assert(vim.wait(5000, function()
    local claim = require("domux").state.claim
    return claim ~= nil and not claim.pending
  end, 10), "the claim was never answered")
end

-- Moves in `dir` and asserts that Neovim changed window, which sends nothing to domux.
function T.move(dir)
  local before = vim.fn.winnr()
  require("domux").navigate(dir)
  assert(vim.fn.winnr() ~= before, "navigate(" .. dir .. ") did not change window")
end

-- Moves in `dir` and waits for the move to reach domux and be answered.
function T.hand_back(dir)
  local domux = require("domux")
  local count = domux.state.last and domux.state.last.n or 0
  local before = vim.fn.winnr()
  domux.navigate(dir)
  assert(vim.fn.winnr() == before, "navigate(" .. dir .. ") changed window instead")
  assert(vim.wait(5000, function()
    return domux.state.last ~= nil and domux.state.last.n == count + 1
  end, 10), "navigate(" .. dir .. ") never reached domux")
end

-- The text :checkhealth domux writes.
function T.health()
  vim.cmd("checkhealth domux")
  return table.concat(vim.api.nvim_buf_get_lines(0, 0, -1, false), "\n")
end

-- The messages the plugin gave the reader, in order.
function T.messages()
  return vim.tbl_map(function(n)
    return n.message
  end, _G.notifications)
end
