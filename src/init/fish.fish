# config.fish: aws-profile-switcher init fish | source
function {{NAME}}
    switch "$argv[1]"
        case ls current exec init help -h --help -V --version
            command {{EXE}} $argv; return $status
    end
    set -l __aps_out (AWS_PROFILE_SWITCHER_SHELL=fish command {{EXE}} $argv); or return $status
    string join \n -- $__aps_out | source
end
