-- `last` goes back where the last move came from: to domux when Neovim has no window to go back
-- to, to a window after a move between windows, and to domux after a move that reached it.
T.claimed()
T.hand_back("last")
vim.cmd("vsplit")
T.move("right")
T.move("last")
T.hand_back("left")
T.hand_back("last")
