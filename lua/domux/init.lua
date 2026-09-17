-- Moves between Neovim windows and domux panes with the same keys (docs/nvim.md).
--
-- In a domux pane, domux gives C-h, C-j, C-k, C-l and C-\ to this Neovim once it has claimed
-- them, and this module hands focus back to domux when there is no window to move to. The
-- claim holds while Neovim is in front of its pane, so a suspended or finished Neovim gives the
-- keys back without saying so (decision 0054).

local M = {}

-- What :checkhealth domux reports and what the tests wait on. Only this module writes it.
M.state = {
  -- nil until a claim is made, then { pending = true }, then { ok = true } or
  -- { ok = false, message = "..." }.
  claim = nil,
  -- nil until a move reaches domux, then { n, method, at, ok, message }. n counts the moves that
  -- reached domux.
  last = nil,
}

local directions = {
  left = { wincmd = "h", method = "focus.left" },
  down = { wincmd = "j", method = "focus.down" },
  up = { wincmd = "k", method = "focus.up" },
  right = { wincmd = "l", method = "focus.right" },
  last = { wincmd = "p", method = "focus.last" },
}

-- Whether the last move went to domux, which is where `last` goes back to.
local went_to_domux = false

local warned = {}

local function warn_once(key, message)
  if warned[key] then
    return
  end
  warned[key] = true
  vim.notify("domux: " .. message .. ". Run :checkhealth domux.", vim.log.levels.WARN)
end

--- Whether this Neovim runs in a domux pane and can talk to its server.
function M.in_pane()
  return vim.fn.has("nvim-0.10") == 1
    and (vim.env.DOMUX_PANE or "") ~= ""
    and (vim.env.DOMUX_SOCKET or "") ~= ""
end

--- Sends one request to the domux server and calls done(err, result) on the main loop. err is
--- nil or { message = "...", socket = true when the server could not be reached }.
---
--- One connection per request, closed after the answer. A claim does not depend on a
--- connection, and one held open would hold up `domux server upgrade` (decision 0046).
function M.request(method, params, done)
  -- Read on the main loop: the callbacks below run where Neovim forbids reading `vim.env`.
  local socket = vim.env.DOMUX_SOCKET
  local pipe = vim.uv.new_pipe(false)
  local finished = false
  local function finish(err, result)
    if finished then
      return
    end
    finished = true
    if not pipe:is_closing() then
      pipe:close()
    end
    vim.schedule(function()
      done(err, result)
    end)
  end
  pipe:connect(socket, function(connect_err)
    if connect_err then
      return finish({
        socket = true,
        message = "the server at " .. socket .. " did not answer: " .. connect_err,
      })
    end
    local buffer = ""
    pipe:read_start(function(read_err, chunk)
      if read_err then
        return finish({ socket = true, message = "reading the answer to " .. method .. " failed: " .. read_err })
      end
      if not chunk then
        return finish({ socket = true, message = "the server closed the connection without answering " .. method })
      end
      buffer = buffer .. chunk
      local newline = buffer:find("\n", 1, true)
      if not newline then
        return
      end
      local ok, response = pcall(vim.json.decode, buffer:sub(1, newline - 1))
      if not ok or type(response) ~= "table" then
        return finish({ message = "the answer to " .. method .. " is not JSON" })
      end
      if type(response.error) == "table" then
        return finish({ message = tostring(response.error.message) })
      end
      finish(nil, response.result)
    end)
    pipe:write(vim.json.encode({ id = 1, method = method, params = params }) .. "\n")
  end)
end

local function pane_params()
  return { pane = vim.env.DOMUX_PANE }
end

--- Asks domux for the passthrough keys in this pane. plugin/domux.lua calls it at startup.
function M.claim()
  M.state.claim = { pending = true }
  M.request("pane.claim_passthrough", pane_params(), function(err)
    if err then
      M.state.claim = { ok = false, message = err.message }
      warn_once(err.socket and "socket" or "claim", "domux keeps C-h, C-j, C-k and C-l in this pane: " .. err.message)
    else
      M.state.claim = { ok = true }
    end
  end)
end

--- Gives the passthrough keys back. plugin/domux.lua calls it when Neovim exits and waits at
--- most 200 ms: a claim stops holding once Neovim is gone, so the release only tidies up.
function M.release()
  local answered = false
  M.request("pane.release_passthrough", pane_params(), function()
    answered = true
  end)
  vim.wait(200, function()
    return answered
  end, 10)
end

local function hand_back(method)
  M.request(method, pane_params(), function(err)
    M.state.last = {
      n = (M.state.last and M.state.last.n or 0) + 1,
      method = method,
      at = os.date("%H:%M:%S"),
      ok = err == nil,
      message = err and err.message,
    }
    if err then
      warn_once(err.socket and "socket" or "move", "focus did not move to the next pane: " .. err.message)
    end
  end)
end

local function wincmd(key)
  local before = vim.fn.winnr()
  pcall(vim.cmd.wincmd, key)
  return vim.fn.winnr() ~= before
end

--- Moves to the window in `dir`, or hands focus to domux when there is none. `dir` is "left",
--- "down", "up", "right" or "last". Outside domux it only moves between windows.
function M.navigate(dir)
  local d = directions[dir]
  if not d then
    error("domux: navigate takes left, down, up, right or last, not " .. tostring(dir))
  end
  if not M.in_pane() then
    wincmd(d.wincmd)
    return
  end
  if not (dir == "last" and went_to_domux) and wincmd(d.wincmd) then
    went_to_domux = false
    return
  end
  went_to_domux = true
  hand_back(d.method)
end

--- Maps C-h, C-j, C-k, C-l and C-\ in normal mode, for plugin managers other than lazy.nvim.
function M.setup()
  local keys = {
    ["<C-h>"] = "left",
    ["<C-j>"] = "down",
    ["<C-k>"] = "up",
    ["<C-l>"] = "right",
    ["<C-\\>"] = "last",
  }
  for lhs, dir in pairs(keys) do
    vim.keymap.set("n", lhs, function()
      M.navigate(dir)
    end, { silent = true, desc = "domux: window or pane " .. dir })
  end
end

return M
