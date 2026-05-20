_k2pdfopt() {
    local i cur prev opts cmd
    COMPREPLY=()
    if [[ "${BASH_VERSINFO[0]}" -ge 4 ]]; then
        cur="$2"
    else
        cur="${COMP_WORDS[COMP_CWORD]}"
    fi
    prev="$3"
    cmd=""
    opts=""

    for i in "${COMP_WORDS[@]:0:COMP_CWORD}"
    do
        case "${cmd},${i}" in
            ",$1")
                cmd="k2pdfopt"
                ;;
            *)
                ;;
        esac
    done

    case "${cmd}" in
        k2pdfopt)
            opts="-o -p -m -t -j -w -x -y -v -h -V --dev --output --pages --px --margins --om --c --no-c --trim --no-t --fc --no-fc --wrap --wrap-extra --no-wrap --ls --ls-pages --no-ls --justify --dpi --odpi --width --height --exit --no-x --yes --no-y --verbose --ui- --ui --ocr --ocr-mode --ocr-visibility-flags --ocr-min-confidence --reflow --list-devices --echo-cmd --dry-run --compat-report --help --version [FILES]..."
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 1 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --dev)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --output)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -o)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --pages)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -p)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --px)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --margins)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --om)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --ls-pages)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --justify)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -j)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --dpi)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --odpi)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --width)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -w)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --height)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --ocr)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --ocr-mode)
                    COMPREPLY=($(compgen -W "strict partial fallback" -- "${cur}"))
                    return 0
                    ;;
                --ocr-visibility-flags)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --ocr-min-confidence)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reflow)
                    COMPREPLY=($(compgen -W "off auto force" -- "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
    esac
}

if [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -ge 4 || "${BASH_VERSINFO[0]}" -gt 4 ]]; then
    complete -F _k2pdfopt -o nosort -o bashdefault -o default k2pdfopt
else
    complete -F _k2pdfopt -o bashdefault -o default k2pdfopt
fi
