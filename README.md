# domux

**terminal runtime for scaling coding agents.**

domux allows you to scale running multiple coding agents. Parallel work comes from Projects and Workspaces. Projects is a git repo, and Workspaces are long running git worktrees in that repo. Long running means, you don't have to manage their lifecycle, and gives you the ability to deploy as many parallel agents as you want. domux currently natively works with claude code, codex and open code. Agent sessions are automatically organised within projects and workspaces, so you can easily peek which agent is blocked on you.

domux will keep your terminal running in a background server so when you close your laptop you don't loose your work. The multiplexing works similar to tmux (that's where the name comes from), but is natively built on top of ghostty.

<img width="1381" height="915" alt="image" src="https://github.com/user-attachments/assets/4bcfa417-7f63-47a0-84ea-99861a21ebe9" />

