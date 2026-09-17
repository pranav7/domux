-- :checkhealth domux

local M = {}

-- Asks the server for server.info and waits up to a second for the answer.
local function ask_server()
  local answer
  require("domux").request("server.info", vim.empty_dict(), function(err, result)
    answer = { err = err, result = result }
  end)
  vim.wait(1000, function()
    return answer ~= nil
  end, 10)
  return answer
end

function M.check()
  if vim.fn.has("nvim-0.10") == 0 then
    vim.health.report_start("domux")
    vim.health.report_error("The plugin needs Neovim 0.10 or later")
    return
  end
  local domux = require("domux")
  vim.health.start("domux")
  vim.health.ok("Neovim " .. tostring(vim.version()))

  if not domux.in_pane() then
    vim.health.info("Not in a domux pane, so C-h, C-j, C-k and C-l move between windows only")
    return
  end
  vim.health.ok("In domux pane " .. vim.env.DOMUX_PANE)

  local answer = ask_server()
  if answer == nil then
    vim.health.error("The server at " .. vim.env.DOMUX_SOCKET .. " did not answer within a second")
  elseif answer.err then
    vim.health.error(answer.err.message)
  else
    vim.health.ok("The server answers, version " .. tostring(answer.result.version))
  end

  local claim = domux.state.claim
  if claim == nil then
    vim.health.warn("No claim was made, so domux keeps C-h, C-j, C-k and C-l in this pane", {
      "Load the plugin at startup: set lazy = false in its lazy.nvim spec",
    })
  elseif claim.pending then
    vim.health.warn("The claim has not been answered yet")
  elseif claim.ok then
    vim.health.ok("domux passes C-h, C-j, C-k, C-l and C-\\ to this Neovim")
  else
    vim.health.error("domux refused the claim: " .. claim.message, {
      "Use the plugin from the same release as your domux binary",
    })
  end

  local last = domux.state.last
  if last == nil then
    vim.health.info("No move has reached domux yet. Press C-h in the leftmost window, then run :checkhealth domux again")
  elseif last.ok then
    vim.health.ok(("Last move to reach domux: %s at %s (%d so far)"):format(last.method, last.at, last.n))
  else
    vim.health.error(("Last move to reach domux: %s at %s failed: %s"):format(last.method, last.at, last.message))
  end
end

return M
