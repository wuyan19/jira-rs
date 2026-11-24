# Regression test for the output encoding of the `jira` CLI (src/output.rs).
#
# Windows PowerShell decodes what a native program writes to a pipe with the
# console output code page, so with a 936 (zh-CN) or 437 (en-US) console the
# UTF-8 `jira` prints would come out as mojibake. The CLI therefore encodes
# its output for PowerShell. This checks that Chinese text reaches PowerShell
# without any shell configuration, and that UTF-8 consumers are left alone.
#
# The CLI's own messages are English, so the Chinese text comes from where it
# does in real use: the Jira data. A tiny fake Jira is started on localhost
# for the duration of the run.
#
#   scripts/test-output-encoding.ps1 -Jira <jira.exe> -Work <scratch dir>
#
# ASCII only: Windows PowerShell 5.1 reads BOM-less scripts as the ANSI code
# page, so the Chinese test data is written as \u escapes below.
# Exit code 0 = all checks passed.
param(
    [Parameter(Mandatory = $true)][string]$Jira,
    [Parameter(Mandatory = $true)][string]$Work
)

$ErrorActionPreference = 'Continue'   # native stderr must reach the pipeline
$Jira = (Resolve-Path $Jira).Path
New-Item -ItemType Directory -Force -Path $Work | Out-Null
$Work = (Resolve-Path $Work).Path
$script:pass = 0; $script:fail = 0; $script:skip = 0
$log = New-Object System.Collections.Generic.List[string]

function Check([string]$Name, [bool]$Ok, [string]$Detail) {
    if ($Ok) { $script:pass++; $log.Add("PASS  $Name") }
    else { $script:fail++; $log.Add("FAIL  $Name  --  $Detail") }
}
function Skip([string]$Name, [string]$Why) { $script:skip++; $log.Add("SKIP  $Name  --  $Why") }
function Run([string[]]$JiraArgs, [switch]$MergeStderr) {
    if ($MergeStderr) { & $Jira @JiraArgs 2>&1 | ForEach-Object { "$_" } }
    else { & $Jira @JiraArgs }
}
function StrictUtf8([byte[]]$Bytes) {
    try { return [Text.Encoding]::GetEncoding('utf-8', [Text.EncoderFallback]::ExceptionFallback, [Text.DecoderFallback]::ExceptionFallback).GetString($Bytes) }
    catch { return $null }
}
function ConsoleCanEncode([string]$Text) {
    $enc = [Console]::OutputEncoding
    return $enc.GetString($enc.GetBytes($Text)) -ceq $Text
}
# The stdout of `jira <args>` as raw bytes, read through a pipe.
function RawOutput([string[]]$JiraArgs) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $Jira
    $psi.Arguments = ($JiraArgs | ForEach-Object { '"' + $_ + '"' }) -join ' '
    $psi.UseShellExecute = $false; $psi.RedirectStandardOutput = $true
    $p = [System.Diagnostics.Process]::Start($psi)
    $bytes = New-Object IO.MemoryStream
    $p.StandardOutput.BaseStream.CopyTo($bytes); $p.WaitForExit()
    return ,($bytes.ToArray())
}
# `$null` when the output is not valid JSON (e.g. mojibake in a broken build):
# one bad result must not abort the remaining checks.
function ParseJson([object[]]$Lines) {
    try { return ($Lines | ConvertFrom-Json) } catch { return $null }
}

# ---- test data ---------------------------------------------------------------
$displayName = [regex]::Unescape('\u6d4b\u8bd5\u7528\u6237')                         # test user
$summary = [regex]::Unescape('\u4e2d\u6587\u6807\u9898\u6d4b\u8bd5')                 # Chinese title, test
$heading = [regex]::Unescape('\u6807\u9898')
$body = [regex]::Unescape('\u6b63\u6587')
$bold = [regex]::Unescape('\u52a0\u7c97')
$serverError = [regex]::Unescape('\u8bf7\u6c42\u7684\u5de5\u5355\u4e0d\u5b58\u5728')  # the requested issue does not exist
# Description as Jira v2 stores it (wiki markup), and what `jira get` renders.
$description = 'h1. ' + $heading + "`n" + $body + ' *' + $bold + '*'
$markdown = '# ' + $heading + "`n" + $body + ' **' + $bold + '**'
# Neither emoji nor this CJK extension B character exist in a legacy code page.
$unencodable = [string][char]0x6D4B + [char]0x8BD5 + ' ' + [char]::ConvertFromUtf32(0x1F680) + ' ' + [char]::ConvertFromUtf32(0x2000B)

$original = [Console]::OutputEncoding
$startCodePage = $original.CodePage
$savedEnv = @{}
$job = $null
try {
    # ---- a tiny fake Jira on localhost ------------------------------------------
    $probe = New-Object System.Net.Sockets.TcpListener([System.Net.IPAddress]::Loopback, 0)
    $probe.Start(); $port = $probe.LocalEndpoint.Port; $probe.Stop()
    $routesPath = Join-Path $Work 'routes.tsv'
    $routes = @(
        @('200', '/rest/api/2/serverInfo', '{"version":"9.12.0","deploymentType":"Server"}'),
        @('200', '/rest/api/2/myself', ('{"name":"tester","displayName":"' + $displayName + '","emailAddress":"tester@example.com","active":true}')),
        @('200', '/rest/api/2/issue/X-1', ('{"id":"1","key":"X-1","fields":{"summary":"' + $summary + '","description":"h1. ' + $heading + '\n' + $body + ' *' + $bold + '*"}}')),
        @('400', '/rest/api/2/issue/BAD-1', ('{"errorMessages":["' + $serverError + '"]}'))
    )
    $lines = @($routes | ForEach-Object { $_ -join "`t" })
    [IO.File]::WriteAllText($routesPath, ($lines -join "`n"), (New-Object Text.UTF8Encoding($false)))

    # status <TAB> path <TAB> body; the body is JSON encoded in UTF-8.
    $job = Start-Job -ArgumentList $port, $routesPath -ScriptBlock {
        param($port, $routesPath)
        $routes = @([IO.File]::ReadAllLines($routesPath, [Text.Encoding]::UTF8) | Where-Object { $_ } | ForEach-Object {
                $parts = $_ -split "`t", 3
                [pscustomobject]@{ Status = [int]$parts[0]; Path = $parts[1]; Body = $parts[2] }
            })
        $listener = New-Object System.Net.HttpListener
        $listener.Prefixes.Add("http://127.0.0.1:$port/")
        $listener.Start()
        while ($listener.IsListening) {
            $ctx = $listener.GetContext()
            $status = 404; $body = '{"errorMessages":["no such route"]}'
            foreach ($route in $routes) {
                if ($route.Path -ceq $ctx.Request.Url.AbsolutePath) { $status = $route.Status; $body = $route.Body }
            }
            $bytes = [Text.Encoding]::UTF8.GetBytes($body)
            $ctx.Response.StatusCode = $status
            $ctx.Response.ContentType = 'application/json'
            $ctx.Response.ContentLength64 = $bytes.Length
            $ctx.Response.OutputStream.Write($bytes, 0, $bytes.Length)
            $ctx.Response.Close()
        }
    }
    $ready = $false
    foreach ($i in 1..60) {
        if ($job.State -eq 'Failed') { break }
        $client = New-Object System.Net.Sockets.TcpClient
        try { $client.Connect('127.0.0.1', $port); $ready = $true }
        catch { Start-Sleep -Milliseconds 250 }
        finally { $client.Close() }
        if ($ready) { break }
    }
    Check 'the fake Jira is listening' $ready ((Receive-Job $job -ErrorAction SilentlyContinue | Out-String))

    # Environment variables take precedence over the config file: clear them
    # so that nothing but the fake Jira is ever contacted.
    foreach ($name in 'JIRA_URL', 'JIRA_USER', 'JIRA_EMAIL', 'JIRA_API_TOKEN', 'JIRA_AUTH_TYPE', 'JIRA_OUTPUT_ENCODING', 'JIRA_TIMEOUT_SECS') {
        $savedEnv[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
        [Environment]::SetEnvironmentVariable($name, $null, 'Process')
    }
    $config = Join-Path $Work 'config.toml'
    [IO.File]::WriteAllText($config, "url = `"http://127.0.0.1:$port`"`nauth_type = `"basic`"`nuser = `"tester`"`ntoken = `"secret`"`n")
    $env:JIRA_CONFIG_FILE = $config

    # ---- reference: the UTF-8 console a user could set up by hand ---------------
    [Console]::OutputEncoding = [Text.Encoding]::UTF8
    $utf8CodePage = [Console]::OutputEncoding.CodePage
    $reference = ParseJson (Run @('get', 'X-1'))
    $truthSummary = $reference.issue.fields.summary
    $truthMarkdown = $reference.descriptionMarkdown
    $truthError = (Run @('get', 'BAD-1') -MergeStderr) -join "`n"

    # ---- the console under test --------------------------------------------------
    # A stock PowerShell decodes with the console output code page (936 on zh-CN).
    # Where the console is already UTF-8 (CI runners), switch it to 936 so that
    # the conversion under test is really exercised.
    $codePage = if ($startCodePage -eq 65001) { 936 } else { $startCodePage }
    [Console]::OutputEncoding = [Text.Encoding]::GetEncoding($codePage)
    $testEncoding = [Console]::OutputEncoding
    $log.Add("PowerShell $($PSVersionTable.PSVersion); console code page under test $([Console]::OutputEncoding.CodePage), wanted $codePage (script started at $startCodePage)")
    $log.Add("reference: summary=$truthSummary; markdown=$truthMarkdown; error=$truthError")

    Check 'reference run really used a UTF-8 console' ($utf8CodePage -eq 65001) "code page $utf8CodePage"
    Check 'the console under test is the intended non-UTF-8 one' ([Console]::OutputEncoding.CodePage -eq $codePage -and $codePage -ne 65001) "code page $([Console]::OutputEncoding.CodePage), wanted $codePage"
    Check 'reference output is the expected Chinese text' ($truthSummary -ceq $summary -and $truthMarkdown -ceq $markdown) "summary '$truthSummary', markdown '$truthMarkdown'"

    # ---- nothing configured: what a user sees by default -------------------------
    $obj = ParseJson (Run @('get', 'X-1'))
    Check 'plain run shows Chinese correctly' ($obj.issue.fields.summary -ceq $truthSummary -and $obj.descriptionMarkdown -ceq $truthMarkdown) "got '$($obj.issue.fields.summary)' / '$($obj.descriptionMarkdown)'"

    $dry = ParseJson (Run @('create', 'X', 'Task', $unencodable, '--dry-run'))
    Check 'JSON stays lossless for characters the code page lacks' ($dry.body.fields.summary -ceq $unencodable) "got '$($dry.body.fields.summary)'"

    if (ConsoleCanEncode $truthError) {
        $err = (Run @('get', 'BAD-1') -MergeStderr) -join "`n"
        Check 'Chinese diagnostics via 2>&1 survive' ($err -ceq $truthError) "got '$err'"
    }
    else {
        Skip 'Chinese diagnostics via 2>&1 survive' "code page $codePage cannot represent them; diagnostics are lossy there by design"
    }

    # What PowerShell reads must be whatever its own encoding decodes correctly.
    [byte[]]$wire = RawOutput @('get', 'X-1')
    $wireJson = ParseJson ([Console]::OutputEncoding.GetString($wire))
    Check 'the bytes on the pipe decode with the console code page' ($wireJson.issue.fields.summary -ceq $truthSummary) "decoded as '$($wireJson.issue.fields.summary)'"

    $f = Join-Path $Work 'redirect.txt'
    Run @('get', 'X-1') > $f
    Check 'PowerShell > redirect keeps the text' ((Get-Content $f -Raw).Contains($truthSummary)) 'summary text not found'

    $f = Join-Path $Work 'to-file.txt'
    Start-Process -FilePath $Jira -ArgumentList 'get', 'X-1' -RedirectStandardOutput $f -NoNewWindow -Wait
    $text = StrictUtf8 ([IO.File]::ReadAllBytes($f))
    Check 'output redirected to a file stays UTF-8' ($null -ne $text -and $text.Contains($truthSummary)) 'file is not UTF-8 with the expected text'

    $f = Join-Path $Work 'via-cmd.txt'
    cmd /c "`"$Jira`" get X-1 > `"$f`""
    $text = StrictUtf8 ([IO.File]::ReadAllBytes($f))
    Check 'a cmd.exe parent gets UTF-8' ($null -ne $text -and $text.Contains($truthSummary)) 'file is not UTF-8 with the expected text'

    $f = Join-Path $Work 'via-bash.txt'
    if (Get-Command bash -ErrorAction SilentlyContinue) {
        & bash -c "'$($Jira.Replace('\', '/'))' get X-1 | cat > '$($f.Replace('\', '/'))'"
        if ($LASTEXITCODE -eq 0 -and (Test-Path $f)) {
            $text = StrictUtf8 ([IO.File]::ReadAllBytes($f))
            Check 'a bash parent gets UTF-8 (even when PowerShell started the bash)' ($null -ne $text -and $text.Contains($truthSummary)) 'file is not UTF-8 with the expected text'
        }
        else {
            Skip 'a bash parent gets UTF-8 (even when PowerShell started the bash)' "bash exited $LASTEXITCODE"
        }
    }
    else {
        Skip 'a bash parent gets UTF-8 (even when PowerShell started the bash)' 'bash not available'
    }

    # MCP must stay UTF-8 even though PowerShell is the parent process here.
    # test_auth returns the user the fake Jira reports: a Chinese display name.
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $Jira; $psi.Arguments = 'mcp'; $psi.UseShellExecute = $false
    $psi.RedirectStandardInput = $true; $psi.RedirectStandardOutput = $true
    $p = [System.Diagnostics.Process]::Start($psi)
    $p.StandardInput.Write('{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}' + "`n" +
        '{"jsonrpc":"2.0","method":"notifications/initialized"}' + "`n" +
        '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"test_auth","arguments":{}}}' + "`n")
    $p.StandardInput.Close()
    $bytes = New-Object IO.MemoryStream
    $p.StandardOutput.BaseStream.CopyTo($bytes); $p.WaitForExit()
    $mcp = StrictUtf8 ($bytes.ToArray())
    Check 'the MCP transport stays raw UTF-8 under a PowerShell parent' ($null -ne $mcp -and $mcp.Contains($displayName)) 'MCP output is not UTF-8 with the expected text'

    # ---- the knobs ---------------------------------------------------------------
    $env:JIRA_OUTPUT_ENCODING = 'utf-8'
    $forced = $null
    $forced = (ParseJson (Run @('get', 'X-1'))).issue.fields.summary
    Check 'JIRA_OUTPUT_ENCODING=utf-8 turns the conversion off' ($forced -cne $truthSummary) "still converted: '$forced'"
    Remove-Item Env:JIRA_OUTPUT_ENCODING -ErrorAction SilentlyContinue

    [Console]::OutputEncoding = [Text.Encoding]::UTF8
    $utf8Console = (ParseJson (Run @('get', 'X-1'))).issue.fields.summary
    [Console]::OutputEncoding = $testEncoding
    Check 'a UTF-8 console keeps working' ($utf8Console -ceq $truthSummary) "got '$utf8Console'"

    [Console]::OutputEncoding = $original
    Check 'console code page restored' ([Console]::OutputEncoding.CodePage -eq $startCodePage) "now $([Console]::OutputEncoding.CodePage), started at $startCodePage"
}
finally {
    if ($job) {
        Stop-Job $job -ErrorAction SilentlyContinue
        Remove-Job $job -Force -ErrorAction SilentlyContinue
    }
    [Console]::OutputEncoding = $original
    foreach ($name in @($savedEnv.Keys)) {
        [Environment]::SetEnvironmentVariable($name, $savedEnv[$name], 'Process')
    }
}

$log | ForEach-Object { Write-Output $_ }
Write-Output ("{0} passed, {1} failed, {2} skipped" -f $script:pass, $script:fail, $script:skip)
if ($script:fail) { exit 1 }
