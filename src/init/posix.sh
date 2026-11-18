# bash: eval "$(aws-profile-switcher init bash)"   zsh: source <(aws-profile-switcher init zsh)
{{NAME}}() {
  case "${1-}" in
    ls|current|exec|init|help|-h|--help|-V|--version) command {{EXE}} "$@"; return ;;
  esac
  local __aps_out
  __aps_out="$(AWS_PROFILE_SWITCHER_SHELL={{SHELL}} command {{EXE}} "$@")" || return
  eval "$__aps_out"
}
