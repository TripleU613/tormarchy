# Managed by tormarchy. Bash completion.
#
# Completes subcommands and their verbs, plus the two things worth completing
# from live state: browsers that are actually installed, and country codes that
# actually have exit relays right now.

_tormarchy() {
  local cur cword
  cur=${COMP_WORDS[COMP_CWORD]}
  cword=$COMP_CWORD

  local subs="connect disconnect mode status ip newnym exit bridge browser
              speed pingd doctor panic boot setup uninstall help"

  if (( cword == 1 )); then
    mapfile -t COMPREPLY < <(compgen -W "$subs" -- "$cur")
    return
  fi

  case "${COMP_WORDS[1]}" in
  connect)   mapfile -t COMPREPLY < <(compgen -W "--dry-run lan strict socks" -- "$cur") ;;
  mode)      mapfile -t COMPREPLY < <(compgen -W "lan strict socks" -- "$cur") ;;
  bridge)    mapfile -t COMPREPLY < <(compgen -W "status set check on off transports" -- "$cur") ;;
  boot)      mapfile -t COMPREPLY < <(compgen -W "status enable disable" -- "$cur") ;;
  status | speed | ip | doctor)
             mapfile -t COMPREPLY < <(compgen -W "--json" -- "$cur") ;;
  uninstall) mapfile -t COMPREPLY < <(compgen -W "--purge" -- "$cur") ;;
  exit)
    # After --exclude: "none", or countries -- one at a time, comma-joined by
    # the user. Only ask Tor when it is running. Otherwise offer the answers
    # that are always valid, rather than hanging on a control port that is not
    # there.
    local first
    if (( cword == 2 )); then
      first="auto --list --exclude"
    elif (( cword == 3 )) && [[ ${COMP_WORDS[2]} == --exclude ]]; then
      first="none"
    else
      return
    fi
    if systemctl is-active --quiet tor.service 2>/dev/null; then
      mapfile -t COMPREPLY < <(compgen -W "$first $(tormarchy exit --list 2>/dev/null | tr '\n' ' ')" -- "$cur")
    else
      mapfile -t COMPREPLY < <(compgen -W "$first" -- "$cur")
    fi
    ;;
  browser)
    mapfile -t COMPREPLY < <(compgen -W "--list $(tormarchy browser --list 2>/dev/null | awk '{print $1}' | tr '\n' ' ')" -- "$cur")
    ;;
  esac
}

complete -F _tormarchy tormarchy
