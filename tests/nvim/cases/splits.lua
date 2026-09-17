-- Between windows the keys stay in Neovim and nothing reaches domux.
T.claimed()
vim.cmd("vsplit")
T.move("right")
T.move("left")
vim.cmd("split")
T.move("down")
T.move("up")
