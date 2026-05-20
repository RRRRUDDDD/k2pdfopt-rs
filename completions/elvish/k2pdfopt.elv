
use builtin;
use str;

set edit:completion:arg-completer[k2pdfopt] = {|@words|
    fn spaces {|n|
        builtin:repeat $n ' ' | str:join ''
    }
    fn cand {|text desc|
        edit:complex-candidate $text &display=$text' '(spaces (- 14 (wcswidth $text)))$desc
    }
    var command = 'k2pdfopt'
    for word $words[1..-1] {
        if (str:has-prefix $word '-') {
            break
        }
        set command = $command';'$word
    }
    var completions = [
        &'k2pdfopt'= {
            cand --dev 'Device profile name or alias (e.g. kv, kpw, ko2, kobo)'
            cand -o 'Output file name format (%s = source name)'
            cand --output 'Output file name format (%s = source name)'
            cand -p 'Page range to process (e.g. 1-10, 1,3,5, even, odd)'
            cand --pages 'Page range to process (e.g. 1-10, 1,3,5, even, odd)'
            cand --px 'Page range to exclude'
            cand -m 'Source crop margins: comma-separated L,T,R,B values (inches default). Single value applies to all four. Suffix: s=source, t=trimmed'
            cand --margins 'Source crop margins: comma-separated L,T,R,B values (inches default). Single value applies to all four. Suffix: s=source, t=trimmed'
            cand --om 'Output margins (same format as -m)'
            cand --ls-pages 'Landscape mode for specific pages'
            cand -j 'Justification: 0=left, 1=center. Suffix + for full-justify, - for no'
            cand --justify 'Justification: 0=left, 1=center. Suffix + for full-justify, - for no'
            cand --dpi 'Output DPI (also sets input DPI)'
            cand --odpi 'Output DPI only'
            cand -w 'Output width with optional unit suffix (px/in/cm/s/t)'
            cand --width 'Output width with optional unit suffix (px/in/cm/s/t)'
            cand --height 'Output height with optional unit suffix (px/in/cm/s/t)'
            cand --ocr 'OCR language(s) for tesseract (e.g. eng, chi_sim, chi_sim+eng). Use "off" to explicitly disable. When omitted, OCR stays off (default)'
            cand --ocr-mode 'OCR 缺语言策略：strict=缺即报错 / partial=丢失保留命中 / fallback=自动落 eng (默认 = v0.1.0 行为)。'
            cand --ocr-visibility-flags 'OCR 输出可见性 bit mask（C `dst_ocr_visibility_flags`）。 bit 0x01=show source bitmap / 0x02=show OCR text (Tr 3 invisible) / 0x04=show boxes / 0x08=use spaces / 0x10=optimized spaces。 默认 1 = SHOW_SOURCE。常用值：3 = source+text, 5 = source+boxes, 7 = source+text+boxes（端到端验收命令）。'
            cand --ocr-min-confidence 'OCR word 置信度过滤阈值 [0.0, 1.0]。低于此值的 word 被丢弃。 默认 0.0 = 不过滤（与 v0.1.0 行为一致）。'
            cand --reflow 'Reflow pipeline mode: off=v0.1.0 直通 / auto=完整 figure+text reflow (默认) / force=即使是单列也跑完整 reflow'
            cand --c 'Color output'
            cand --no-c 'Disable color output'
            cand -t 'Trim source margins'
            cand --trim 'Trim source margins'
            cand --no-t 'Disable trimming'
            cand --fc 'Fit columns to screen width'
            cand --no-fc 'Disable fit-columns'
            cand --wrap 'Enable text wrapping'
            cand --wrap-extra 'Extra text wrapping (C''s -wrap+)'
            cand --no-wrap 'Disable text wrapping'
            cand --ls 'Landscape orientation'
            cand --no-ls 'Disable landscape'
            cand -x 'Exit on complete (C''s -x)'
            cand --exit 'Exit on complete (C''s -x)'
            cand --no-x 'Don''t exit on complete'
            cand -y 'Assume yes to all prompts'
            cand --yes 'Assume yes to all prompts'
            cand --no-y 'Don''t assume yes'
            cand -v 'Verbose output (repeat for more: -v, -vv, -vvv)'
            cand --verbose 'Verbose output (repeat for more: -v, -vv, -vvv)'
            cand --ui- 'Non-interactive mode (C''s -ui-)'
            cand --ui 'Interactive mode (C''s -ui)'
            cand --list-devices 'List all device profiles'
            cand --echo-cmd 'Echo the equivalent command line'
            cand --dry-run 'Show conversion plan without processing'
            cand --compat-report 'Show compatibility report vs. C version'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
            cand -V 'Print version'
            cand --version 'Print version'
        }
    ]
    $completions[$command]
}
