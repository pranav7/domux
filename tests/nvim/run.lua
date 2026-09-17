-- The Neovim plugin's suite (docs/nvim.md, decision 0054). From the repository root:
--
--   nvim --headless -l tests/nvim/run.lua
--
-- Every case starts its own Neovim with the plugin on the runtime path, so what the plugin
-- does at startup and at exit is part of the case. This Neovim serves a fake control API on a
-- Unix socket and records the requests each case sends.

local uv = vim.uv
local root = vim.fn.fnamemodify(debug.getinfo(1, "S").source:sub(2), ":p:h:h:h")
local PANE = "p_test"

-- Every request the plugin may send. crates/domux-core parses this file with the server's own
-- parser, so the fake below cannot accept a request the real server refuses.
local contract = {}
for line in io.lines(root .. "/tests/nvim/requests.jsonl") do
  if line ~= "" then
    table.insert(contract, { request = vim.json.decode(line), sent = false })
  end
end

local function request(method, params)
  return { id = 1, method = method, params = params or vim.empty_dict() }
end

local function pane_request(method)
  return request(method, { pane = PANE })
end

-- A fake server. answers[method] is the response to send, { result = ... } or
-- { error = { code = ..., message = ... } }. A method with no entry answers { result = { ok = true } }.
local function serve(answers)
  local path = os.tmpname()
  os.remove(path)
  path = path .. ".sock"
  local requests = {}
  local server = assert(uv.new_pipe(false))
  assert(server:bind(path))
  assert(server:listen(16, function(listen_err)
    assert(not listen_err, listen_err)
    local conn = assert(uv.new_pipe(false))
    server:accept(conn)
    local buffer = ""
    conn:read_start(function(read_err, chunk)
      if read_err or not chunk then
        conn:close()
        return
      end
      buffer = buffer .. chunk
      local newline = buffer:find("\n", 1, true)
      if not newline then
        return
      end
      local req = vim.json.decode(buffer:sub(1, newline - 1))
      buffer = buffer:sub(newline + 1)
      table.insert(requests, req)
      local response = vim.deepcopy(answers[req.method] or { result = { ok = true } })
      response.id = req.id
      conn:write(vim.json.encode(response) .. "\n")
    end)
  end))
  return path, requests, function()
    server:close()
    os.remove(path)
  end
end

local failed = {}

-- case(name, opts) runs tests/nvim/cases/<name>.lua in a Neovim of its own.
--   opts.env      "pane" for DOMUX_PANE=p_test and DOMUX_SOCKET at the fake, a table of
--                 variables, or nil for no domux variables at all
--   opts.answers  the fake server's answers
--   opts.expect   the requests the case sends, in order
local function case(name, opts)
  local socket, requests, stop = serve(opts.answers or {})
  local home = vim.fn.tempname()
  vim.fn.mkdir(home, "p")
  -- Merged into this Neovim's environment. Neovim 0.10's vim.system drops `env` altogether
  -- under clear_env, so the domux variables a pane gives this Neovim are blanked instead, and
  -- the plugin reads an empty one as unset.
  local env = {
    HOME = home,
    XDG_CONFIG_HOME = home,
    XDG_DATA_HOME = home,
    XDG_STATE_HOME = home,
    DOMUX_PANE = "",
    DOMUX_SOCKET = "",
  }
  if opts.env == "pane" then
    env.DOMUX_PANE = PANE
    env.DOMUX_SOCKET = socket
  elseif type(opts.env) == "table" then
    for k, v in pairs(opts.env) do
      env[k] = v
    end
  end
  local result
  local job = vim.system({
    vim.v.progpath,
    "--headless",
    "--clean",
    "--cmd",
    "set rtp^=" .. vim.fn.fnameescape(root),
    "--cmd",
    "luafile " .. vim.fn.fnameescape(root .. "/tests/nvim/case.lua"),
    "-c",
    "lua T.run(" .. vim.inspect(root .. "/tests/nvim/cases/" .. name .. ".lua") .. ")",
  }, { env = env, text = true }, function(r)
    result = r
  end)
  if not vim.wait(15000, function()
    return result ~= nil
  end, 10) then
    job:kill(9)
    vim.wait(1000, function()
      return result ~= nil
    end, 10)
  end
  stop()

  local problems = {}
  if not result or result.signal ~= 0 then
    table.insert(problems, "did not finish within 15 seconds")
  elseif result.code ~= 0 then
    table.insert(problems, "failed: " .. result.stderr)
  end
  if not vim.deep_equal(requests, opts.expect) then
    table.insert(
      problems,
      "sent " .. vim.inspect(requests) .. "\n  but the case expects " .. vim.inspect(opts.expect)
    )
  end
  for _, req in ipairs(requests) do
    local known = false
    for _, entry in ipairs(contract) do
      if vim.deep_equal(req, entry.request) then
        entry.sent = true
        known = true
      end
    end
    if not known then
      table.insert(problems, "sent a request tests/nvim/requests.jsonl does not hold: " .. vim.json.encode(req))
    end
  end
  if #problems == 0 then
    io.stdout:write("ok   " .. name .. "\n")
  else
    io.stdout:write("FAIL " .. name .. "\n  " .. table.concat(problems, "\n  ") .. "\n")
    table.insert(failed, name)
  end
end

local refused_claim = {
  error = {
    code = "not_found",
    message = "method pane.claim_passthrough does not exist; run domux api schema for the list",
  },
}
local refused_move = {
  error = { code = "not_found", message = "pane p_test does not exist; run domux api pane.list" },
}

case("claim", {
  env = "pane",
  expect = { pane_request("pane.claim_passthrough"), pane_request("pane.release_passthrough") },
})
case("splits", {
  env = "pane",
  expect = { pane_request("pane.claim_passthrough"), pane_request("pane.release_passthrough") },
})
case("edges", {
  env = "pane",
  expect = {
    pane_request("pane.claim_passthrough"),
    pane_request("focus.left"),
    pane_request("focus.down"),
    pane_request("focus.up"),
    pane_request("focus.right"),
    pane_request("pane.release_passthrough"),
  },
})
case("last", {
  env = "pane",
  expect = {
    pane_request("pane.claim_passthrough"),
    pane_request("focus.last"),
    pane_request("focus.left"),
    pane_request("focus.last"),
    pane_request("pane.release_passthrough"),
  },
})
case("health", {
  env = "pane",
  answers = { ["server.info"] = { result = { version = "9.9.9" } } },
  expect = {
    pane_request("pane.claim_passthrough"),
    pane_request("focus.left"),
    request("server.info"),
    pane_request("pane.release_passthrough"),
  },
})
case("refused", {
  env = "pane",
  answers = { ["pane.claim_passthrough"] = refused_claim, ["focus.left"] = refused_move },
  expect = {
    pane_request("pane.claim_passthrough"),
    pane_request("focus.left"),
    pane_request("focus.left"),
    pane_request("focus.left"),
    request("server.info"),
    pane_request("pane.release_passthrough"),
  },
})
case("dead_socket", {
  env = { DOMUX_PANE = PANE, DOMUX_SOCKET = "/nonexistent/domux.sock" },
  expect = {},
})
case("outside", { expect = {} })

for _, entry in ipairs(contract) do
  if not entry.sent then
    table.insert(failed, "tests/nvim/requests.jsonl holds a request no case sent: " .. vim.json.encode(entry.request))
  end
end

if #failed > 0 then
  io.stdout:write(#failed .. " failed:\n  " .. table.concat(failed, "\n  ") .. "\n")
  os.exit(1)
end
io.stdout:write("all passed\n")
