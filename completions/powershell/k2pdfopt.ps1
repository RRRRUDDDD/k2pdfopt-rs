
using namespace System.Management.Automation
using namespace System.Management.Automation.Language

Register-ArgumentCompleter -Native -CommandName 'k2pdfopt' -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

    $commandElements = $commandAst.CommandElements
    $command = @(
        'k2pdfopt'
        for ($i = 1; $i -lt $commandElements.Count; $i++) {
            $element = $commandElements[$i]
            if ($element -isnot [StringConstantExpressionAst] -or
                $element.StringConstantType -ne [StringConstantType]::BareWord -or
                $element.Value.StartsWith('-') -or
                $element.Value -eq $wordToComplete) {
                break
        }
        $element.Value
    }) -join ';'

    $completions = @(switch ($command) {
        'k2pdfopt' {
            [CompletionResult]::new('--dev', '--dev', [CompletionResultType]::ParameterName, 'Device profile name or alias (e.g. kv, kpw, ko2, kobo)')
            [CompletionResult]::new('-o', '-o', [CompletionResultType]::ParameterName, 'Output file name format (%s = source name)')
            [CompletionResult]::new('--output', '--output', [CompletionResultType]::ParameterName, 'Output file name format (%s = source name)')
            [CompletionResult]::new('-p', '-p', [CompletionResultType]::ParameterName, 'Page range to process (e.g. 1-10, 1,3,5, even, odd)')
            [CompletionResult]::new('--pages', '--pages', [CompletionResultType]::ParameterName, 'Page range to process (e.g. 1-10, 1,3,5, even, odd)')
            [CompletionResult]::new('--px', '--px', [CompletionResultType]::ParameterName, 'Page range to exclude')
            [CompletionResult]::new('-m', '-m', [CompletionResultType]::ParameterName, 'Source crop margins: comma-separated L,T,R,B values (inches default). Single value applies to all four. Suffix: s=source, t=trimmed')
            [CompletionResult]::new('--margins', '--margins', [CompletionResultType]::ParameterName, 'Source crop margins: comma-separated L,T,R,B values (inches default). Single value applies to all four. Suffix: s=source, t=trimmed')
            [CompletionResult]::new('--om', '--om', [CompletionResultType]::ParameterName, 'Output margins (same format as -m)')
            [CompletionResult]::new('--ls-pages', '--ls-pages', [CompletionResultType]::ParameterName, 'Landscape mode for specific pages')
            [CompletionResult]::new('-j', '-j', [CompletionResultType]::ParameterName, 'Justification: 0=left, 1=center. Suffix + for full-justify, - for no')
            [CompletionResult]::new('--justify', '--justify', [CompletionResultType]::ParameterName, 'Justification: 0=left, 1=center. Suffix + for full-justify, - for no')
            [CompletionResult]::new('--dpi', '--dpi', [CompletionResultType]::ParameterName, 'Output DPI (also sets input DPI)')
            [CompletionResult]::new('--odpi', '--odpi', [CompletionResultType]::ParameterName, 'Output DPI only')
            [CompletionResult]::new('-w', '-w', [CompletionResultType]::ParameterName, 'Output width with optional unit suffix (px/in/cm/s/t)')
            [CompletionResult]::new('--width', '--width', [CompletionResultType]::ParameterName, 'Output width with optional unit suffix (px/in/cm/s/t)')
            [CompletionResult]::new('--height', '--height', [CompletionResultType]::ParameterName, 'Output height with optional unit suffix (px/in/cm/s/t)')
            [CompletionResult]::new('--ocr', '--ocr', [CompletionResultType]::ParameterName, 'OCR language(s) for tesseract (e.g. eng, chi_sim, chi_sim+eng). Use "off" to explicitly disable. When omitted, OCR stays off (default)')
            [CompletionResult]::new('--ocr-mode', '--ocr-mode', [CompletionResultType]::ParameterName, 'OCR 缺语言策略：strict=缺即报错 / partial=丢失保留命中 / fallback=自动落 eng (默认 = v0.1.0 行为)。')
            [CompletionResult]::new('--ocr-visibility-flags', '--ocr-visibility-flags', [CompletionResultType]::ParameterName, 'OCR 输出可见性 bit mask（C `dst_ocr_visibility_flags`）。 bit 0x01=show source bitmap / 0x02=show OCR text (Tr 3 invisible) / 0x04=show boxes / 0x08=use spaces / 0x10=optimized spaces。 默认 1 = SHOW_SOURCE。常用值：3 = source+text, 5 = source+boxes, 7 = source+text+boxes（端到端验收命令）。')
            [CompletionResult]::new('--ocr-min-confidence', '--ocr-min-confidence', [CompletionResultType]::ParameterName, 'OCR word 置信度过滤阈值 [0.0, 1.0]。低于此值的 word 被丢弃。 默认 0.0 = 不过滤（与 v0.1.0 行为一致）。')
            [CompletionResult]::new('--reflow', '--reflow', [CompletionResultType]::ParameterName, 'Reflow pipeline mode: off=v0.1.0 直通 / auto=完整 figure+text reflow (默认) / force=即使是单列也跑完整 reflow')
            [CompletionResult]::new('--c', '--c', [CompletionResultType]::ParameterName, 'Color output')
            [CompletionResult]::new('--no-c', '--no-c', [CompletionResultType]::ParameterName, 'Disable color output')
            [CompletionResult]::new('-t', '-t', [CompletionResultType]::ParameterName, 'Trim source margins')
            [CompletionResult]::new('--trim', '--trim', [CompletionResultType]::ParameterName, 'Trim source margins')
            [CompletionResult]::new('--no-t', '--no-t', [CompletionResultType]::ParameterName, 'Disable trimming')
            [CompletionResult]::new('--fc', '--fc', [CompletionResultType]::ParameterName, 'Fit columns to screen width')
            [CompletionResult]::new('--no-fc', '--no-fc', [CompletionResultType]::ParameterName, 'Disable fit-columns')
            [CompletionResult]::new('--wrap', '--wrap', [CompletionResultType]::ParameterName, 'Enable text wrapping')
            [CompletionResult]::new('--wrap-extra', '--wrap-extra', [CompletionResultType]::ParameterName, 'Extra text wrapping (C''s -wrap+)')
            [CompletionResult]::new('--no-wrap', '--no-wrap', [CompletionResultType]::ParameterName, 'Disable text wrapping')
            [CompletionResult]::new('--ls', '--ls', [CompletionResultType]::ParameterName, 'Landscape orientation')
            [CompletionResult]::new('--no-ls', '--no-ls', [CompletionResultType]::ParameterName, 'Disable landscape')
            [CompletionResult]::new('-x', '-x', [CompletionResultType]::ParameterName, 'Exit on complete (C''s -x)')
            [CompletionResult]::new('--exit', '--exit', [CompletionResultType]::ParameterName, 'Exit on complete (C''s -x)')
            [CompletionResult]::new('--no-x', '--no-x', [CompletionResultType]::ParameterName, 'Don''t exit on complete')
            [CompletionResult]::new('-y', '-y', [CompletionResultType]::ParameterName, 'Assume yes to all prompts')
            [CompletionResult]::new('--yes', '--yes', [CompletionResultType]::ParameterName, 'Assume yes to all prompts')
            [CompletionResult]::new('--no-y', '--no-y', [CompletionResultType]::ParameterName, 'Don''t assume yes')
            [CompletionResult]::new('-v', '-v', [CompletionResultType]::ParameterName, 'Verbose output (repeat for more: -v, -vv, -vvv)')
            [CompletionResult]::new('--verbose', '--verbose', [CompletionResultType]::ParameterName, 'Verbose output (repeat for more: -v, -vv, -vvv)')
            [CompletionResult]::new('--ui-', '--ui-', [CompletionResultType]::ParameterName, 'Non-interactive mode (C''s -ui-)')
            [CompletionResult]::new('--ui', '--ui', [CompletionResultType]::ParameterName, 'Interactive mode (C''s -ui)')
            [CompletionResult]::new('--list-devices', '--list-devices', [CompletionResultType]::ParameterName, 'List all device profiles')
            [CompletionResult]::new('--echo-cmd', '--echo-cmd', [CompletionResultType]::ParameterName, 'Echo the equivalent command line')
            [CompletionResult]::new('--dry-run', '--dry-run', [CompletionResultType]::ParameterName, 'Show conversion plan without processing')
            [CompletionResult]::new('--compat-report', '--compat-report', [CompletionResultType]::ParameterName, 'Show compatibility report vs. C version')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('-V', '-V ', [CompletionResultType]::ParameterName, 'Print version')
            [CompletionResult]::new('--version', '--version', [CompletionResultType]::ParameterName, 'Print version')
            break
        }
    })

    $completions.Where{ $_.CompletionText -like "$wordToComplete*" } |
        Sort-Object -Property ListItemText
}
