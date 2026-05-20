complete -c k2pdfopt -l dev -d 'Device profile name or alias (e.g. kv, kpw, ko2, kobo)' -r
complete -c k2pdfopt -s o -l output -d 'Output file name format (%s = source name)' -r
complete -c k2pdfopt -s p -l pages -d 'Page range to process (e.g. 1-10, 1,3,5, even, odd)' -r
complete -c k2pdfopt -l px -d 'Page range to exclude' -r
complete -c k2pdfopt -s m -l margins -d 'Source crop margins: comma-separated L,T,R,B values (inches default). Single value applies to all four. Suffix: s=source, t=trimmed' -r
complete -c k2pdfopt -l om -d 'Output margins (same format as -m)' -r
complete -c k2pdfopt -l ls-pages -d 'Landscape mode for specific pages' -r
complete -c k2pdfopt -s j -l justify -d 'Justification: 0=left, 1=center. Suffix + for full-justify, - for no' -r
complete -c k2pdfopt -l dpi -d 'Output DPI (also sets input DPI)' -r
complete -c k2pdfopt -l odpi -d 'Output DPI only' -r
complete -c k2pdfopt -s w -l width -d 'Output width with optional unit suffix (px/in/cm/s/t)' -r
complete -c k2pdfopt -l height -d 'Output height with optional unit suffix (px/in/cm/s/t)' -r
complete -c k2pdfopt -l ocr -d 'OCR language(s) for tesseract (e.g. eng, chi_sim, chi_sim+eng). Use "off" to explicitly disable. When omitted, OCR stays off (default)' -r
complete -c k2pdfopt -l ocr-mode -d 'OCR 缺语言策略：strict=缺即报错 / partial=丢失保留命中 / fallback=自动落 eng (默认 = v0.1.0 行为)。' -r -f -a "strict\t''
partial\t''
fallback\t''"
complete -c k2pdfopt -l ocr-visibility-flags -d 'OCR 输出可见性 bit mask（C `dst_ocr_visibility_flags`）。 bit 0x01=show source bitmap / 0x02=show OCR text (Tr 3 invisible) / 0x04=show boxes / 0x08=use spaces / 0x10=optimized spaces。 默认 1 = SHOW_SOURCE。常用值：3 = source+text, 5 = source+boxes, 7 = source+text+boxes（端到端验收命令）。' -r
complete -c k2pdfopt -l ocr-min-confidence -d 'OCR word 置信度过滤阈值 [0.0, 1.0]。低于此值的 word 被丢弃。 默认 0.0 = 不过滤（与 v0.1.0 行为一致）。' -r
complete -c k2pdfopt -l reflow -d 'Reflow pipeline mode: off=v0.1.0 直通 / auto=完整 figure+text reflow (默认) / force=即使是单列也跑完整 reflow' -r -f -a "off\t''
auto\t''
force\t''"
complete -c k2pdfopt -l c -d 'Color output'
complete -c k2pdfopt -l no-c -d 'Disable color output'
complete -c k2pdfopt -s t -l trim -d 'Trim source margins'
complete -c k2pdfopt -l no-t -d 'Disable trimming'
complete -c k2pdfopt -l fc -d 'Fit columns to screen width'
complete -c k2pdfopt -l no-fc -d 'Disable fit-columns'
complete -c k2pdfopt -l wrap -d 'Enable text wrapping'
complete -c k2pdfopt -l wrap-extra -d 'Extra text wrapping (C\'s -wrap+)'
complete -c k2pdfopt -l no-wrap -d 'Disable text wrapping'
complete -c k2pdfopt -l ls -d 'Landscape orientation'
complete -c k2pdfopt -l no-ls -d 'Disable landscape'
complete -c k2pdfopt -s x -l exit -d 'Exit on complete (C\'s -x)'
complete -c k2pdfopt -l no-x -d 'Don\'t exit on complete'
complete -c k2pdfopt -s y -l yes -d 'Assume yes to all prompts'
complete -c k2pdfopt -l no-y -d 'Don\'t assume yes'
complete -c k2pdfopt -s v -l verbose -d 'Verbose output (repeat for more: -v, -vv, -vvv)'
complete -c k2pdfopt -l ui- -d 'Non-interactive mode (C\'s -ui-)'
complete -c k2pdfopt -l ui -d 'Interactive mode (C\'s -ui)'
complete -c k2pdfopt -l list-devices -d 'List all device profiles'
complete -c k2pdfopt -l echo-cmd -d 'Echo the equivalent command line'
complete -c k2pdfopt -l dry-run -d 'Show conversion plan without processing'
complete -c k2pdfopt -l compat-report -d 'Show compatibility report vs. C version'
complete -c k2pdfopt -s h -l help -d 'Print help (see more with \'--help\')'
complete -c k2pdfopt -s V -l version -d 'Print version'
