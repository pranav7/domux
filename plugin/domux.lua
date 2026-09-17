-- Claims C-h, C-j, C-k, C-l and C-\ for this Neovim when it runs in a domux pane (docs/nvim.md).
-- It maps no key: the mappings are the user's, through lazy.nvim's `keys` or
-- require("domux").setup().

if vim.g.loaded_domux then
  return
end
vim.g.loaded_domux = true

if vim.fn.has("nvim-0.10") == 0 then
  if (vim.env.DOMUX_PANE or "") ~= "" then
    vim.notify(
      "domux: the plugin needs Neovim 0.10 or later, so domux keeps C-h, C-j, C-k and C-l in this pane",
      vim.log.levels.WARN
    )
  end
  return
end

local domux = require("domux")
if not domux.in_pane() then
  return
end

domux.claim()
vim.api.nvim_create_autocmd("VimLeavePre", {
  group = vim.api.nvim_create_augroup("domux", { clear = true }),
  callback = function()
    domux.release()
  end,
})
