-- A server that cannot be reached warns once, for the claim and every move after it.
T.claimed()
T.hand_back("left")
T.hand_back("left")
local messages = T.messages()
assert(#messages == 1, vim.inspect(messages))
assert(messages[1]:find("did not answer", 1, true), messages[1])
