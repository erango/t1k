# T1K shell integration for zsh. T1K points ZDOTDIR here so this file runs first; it puts
# ZDOTDIR back, loads your own .zshenv, and (in interactive shells) marks each prompt so T1K
# can jump between prompts with Cmd-Up / Cmd-Down. Your .zprofile and .zshrc load as usual.
# Turn it off with terminal.shell_integration = false in ~/.config/t1k/config.toml.

if [[ -n "${T1K_ORIG_ZDOTDIR}" ]]; then
  ZDOTDIR="$T1K_ORIG_ZDOTDIR"
  unset T1K_ORIG_ZDOTDIR
else
  unset ZDOTDIR T1K_ORIG_ZDOTDIR
fi

[[ -f "${ZDOTDIR:-$HOME}/.zshenv" ]] && builtin source "${ZDOTDIR:-$HOME}/.zshenv"

if [[ -o interactive ]]; then
  # The prompt and the command line are wrapped in an OSC 8 link to t1k://prompt. Terminals
  # keep links on cells, so the mark survives scrollback and resizes; T1K never shows it.
  _t1k_prompt_start() { builtin print -n '\e]8;;t1k://prompt\e\\' }
  _t1k_command_start() { builtin print -n '\e]8;;\e\\' }
  builtin autoload -Uz add-zsh-hook
  add-zsh-hook precmd _t1k_prompt_start
  add-zsh-hook preexec _t1k_command_start
  # What you type gets its own mark, so T1K can copy the command apart from the prompt.
  _t1k_command_mark() { builtin print -n '\e]8;;t1k://prompt/cmd\e\\' }
  builtin autoload -Uz add-zle-hook-widget
  add-zle-hook-widget line-init _t1k_command_mark
  # Closing a tab hangs up its shell. Save what you typed first, so a reopened tab (or any
  # new one) finds it with ↑.
  TRAPHUP() { [[ -n "$HISTFILE" ]] && builtin fc -AI 2>/dev/null; builtin exit 129 }
fi
