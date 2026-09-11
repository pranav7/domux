<p align="left">
  <img width="100" height="100" alt="image" src="https://github.com/user-attachments/assets/5cafbde2-865b-4f5e-889f-d10d9963a472" />
</p>

# domux

**terminal runtime for scaling coding agents.**

domux allows you to scale running multiple coding agents. Parallel work comes from Projects and Workspaces. A Project is a git repo, and Workspaces are long running git worktrees in that repo. Long running means you don't have to manage their lifecycle, and gives you the ability to deploy as many parallel agents as you want. domux currently natively works with Claude Code, Codex and OpenCode. Agent sessions are automatically organised within projects and workspaces, so you can easily peek at which agent is blocked on you.

domux will keep your terminal running in a background server so when you close your laptop you don't lose your work. The multiplexing works similarly to tmux (that's where the name comes from), but is natively built on top of Ghostty. All your tmux shortcuts should work out the box as well.

<img width="1381" height="915" alt="image" src="https://github.com/user-attachments/assets/4bcfa417-7f63-47a0-84ea-99861a21ebe9" />
