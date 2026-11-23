# Install the jira binary from GitHub Releases (Windows).
#
#   irm https://raw.githubusercontent.com/wuyan19/jira-rs/main/scripts/install.ps1 | iex
#
# Options (parameters, or environment variables of the JIRA_* names):
#   -Version <tag>        e.g. v0.1.1        (default: latest release)
#   -DownloadBase <url>   prefix in front of https://github.com for asset
#                         downloads, e.g. a GitHub accelerator
#   -InstallDir <dir>     default: $LOCALAPPDATA\Programs\jira
#
# Environment: JIRA_VERSION, JIRA_DOWNLOAD_BASE, JIRA_INSTALL_DIR,
#   JIRA_SKIP_VERIFY=1 (skip the checksum, not recommended)
#
# Version lookup always goes to github.com directly: accelerators reliably
# proxy /releases/download/<tag>/<file> but not necessarily the
# /releases/latest redirect.
#
# ASCII only: Windows PowerShell 5.1 reads BOM-less files as the system
# ANSI code page. Compatible with PowerShell 5.1 and 7.
#
# The body lives in a function so that, when run through `iex`, it does not
# leave preference changes behind in the caller's session.

param(
    [string]$Version = $env:JIRA_VERSION,
    [string]$DownloadBase = $env:JIRA_DOWNLOAD_BASE,
    [string]$InstallDir = $env:JIRA_INSTALL_DIR
)

function Install-Jira {
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'   # the 5.1 progress bar makes downloads crawl
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $Repo = 'wuyan19/jira-rs'
    $Github = 'https://github.com'
    $Target = 'x86_64-pc-windows-msvc'
    if (-not $InstallDir) { $InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\jira' }
    # Canonical form for the PATH entry: no forward slashes, no trailing slash.
    if ([System.IO.Path]::IsPathRooted($InstallDir)) {
        $InstallDir = [System.IO.Path]::GetFullPath($InstallDir)
        if ($InstallDir.Length -gt 3) { $InstallDir = $InstallDir.TrimEnd('\') }
    }
    if (-not $DownloadBase) { $DownloadBase = $Github }
    $DownloadBase = $DownloadBase.TrimEnd('/')

    # `exit` would close the caller's shell under `iex`; `throw` does not.
    function Fail([string]$Message) { throw "jira install: $Message" }

    function Save-Url([string]$Url, [string]$OutFile) {
        # One retry for a broken transfer; an error status (404 from an
        # incomplete mirror, 403, ...) would just come back.
        foreach ($attempt in 1..2) {
            try {
                Invoke-WebRequest -Uri $Url -OutFile $OutFile -UseBasicParsing
                return
            } catch {
                $status = if ($_.Exception.Response) { [int]$_.Exception.Response.StatusCode } else { 0 }
                if ($attempt -eq 2 -or ($status -ge 400 -and $status -lt 500)) {
                    Fail "download failed: $Url`n$($_.Exception.Message)"
                }
                Start-Sleep -Seconds 1
            }
        }
    }

    function Get-LatestTag {
        # 1) Redirect target of /releases/latest: one request, no API rate limit.
        $first = ''
        try {
            $request = [System.Net.HttpWebRequest]::Create("$Github/$Repo/releases/latest")
            $request.Method = 'HEAD'
            $request.AllowAutoRedirect = $false
            $request.Timeout = 15000
            $request.UserAgent = 'jira-install'
            $response = $request.GetResponse()
            $location = [string]$response.Headers['Location']
            $response.Close()
            if ($location -match '/releases/tag/(v[\d][^/]*)') { return $Matches[1] }
        } catch {
            $first = $_.Exception.Message
        }

        # 2) REST API (rate-limited per IP, hence second).
        try {
            $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" `
                -Headers @{ Accept = 'application/vnd.github+json'; 'User-Agent' = 'jira-install' } `
                -UseBasicParsing
        } catch {
            Fail "cannot determine the latest release (is github.com reachable?). Set JIRA_VERSION, for example: `$env:JIRA_VERSION = 'v0.1.1'`n$first`n$($_.Exception.Message)"
        }
        return [string]$release.tag_name
    }

    # Keep the registry value kind and unexpanded %VAR% references: writing
    # back the expanded User PATH would freeze them (and break REG_EXPAND_SZ).
    function Add-ToUserPath([string]$Dir) {
        $expanded = [Environment]::ExpandEnvironmentVariables($Dir).TrimEnd('\')
        $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
        $added = $false
        try {
            $raw = [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            $onPath = $false
            foreach ($entry in ($raw -split ';')) {
                if ([Environment]::ExpandEnvironmentVariables($entry).TrimEnd('\') -ieq $expanded) { $onPath = $true }
            }
            if (-not $onPath) {
                $kind = [Microsoft.Win32.RegistryValueKind]::String
                if ($null -ne $key.GetValue('Path', $null) -and $key.GetValueKind('Path') -eq [Microsoft.Win32.RegistryValueKind]::ExpandString) {
                    $kind = [Microsoft.Win32.RegistryValueKind]::ExpandString
                }
                $key.SetValue('Path', ($raw.TrimEnd(';') + ';' + $Dir).TrimStart(';'), $kind)
                $added = $true
            }
        } finally {
            $key.Close()
        }
        return $added
    }

    # Tell running programs (Explorer, so newly opened terminals) to reload
    # the environment. Best effort: a re-login also picks up the change.
    function Update-Environment {
        try {
            if (-not ('JiraInstall.NativeMethods' -as [type])) {
                Add-Type -Namespace JiraInstall -Name NativeMethods -MemberDefinition @'
[DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Auto)]
public static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint Msg, UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out UIntPtr lpdwResult);
'@
            }
            $result = [UIntPtr]::Zero
            [JiraInstall.NativeMethods]::SendMessageTimeout([IntPtr]0xffff, 0x001A, [UIntPtr]::Zero, 'Environment', 2, 5000, [ref]$result) | Out-Null
        } catch {
        }
    }

    # ---- platform and version ------------------------------------------------

    if ($env:OS -ne 'Windows_NT') { Fail 'this script is for Windows; use scripts/install.sh' }
    if (-not $Version) { $Version = Get-LatestTag }
    if ($Version -notmatch '^v') { $Version = "v$Version" }
    if ($Version -notmatch '^v\d+\.\d+\.\d+') { Fail "'$Version' does not look like a release tag" }

    $asset = "jira-$($Version.Substring(1))-$Target.exe"
    $root = "$DownloadBase/$Repo/releases/download/$Version"

    # ---- download and verify -------------------------------------------------

    Write-Host "Installing jira $Version to $InstallDir"
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $tmp = Join-Path $InstallDir "jira.tmp-$PID.exe"
    $sum = "$tmp.sha256"
    try {
        Save-Url "$root/$asset" $tmp

        if ($env:JIRA_SKIP_VERIFY -eq '1') {
            Write-Warning 'JIRA_SKIP_VERIFY=1: skipping checksum verification'
        } else {
            Save-Url "$root/$asset.sha256" $sum
            $want = ((Get-Content -TotalCount 1 $sum) -split '\s+')[0].ToLower()
            $got = (Get-FileHash -Algorithm SHA256 -Path $tmp).Hash.ToLower()
            if ($got -ne $want) {
                Fail "checksum mismatch for $asset (got $got, want $want). The download may be truncated or tampered with; try another JIRA_DOWNLOAD_BASE."
            }
        }

        # ---- install ----------------------------------------------------------
        # Windows cannot overwrite a running .exe but does allow renaming it
        # (e.g. a `jira mcp` process owned by an MCP client).
        $dest = Join-Path $InstallDir 'jira.exe'
        if (Test-Path $dest) {
            $old = "$dest.old"
            Remove-Item $old -Force -ErrorAction SilentlyContinue
            Rename-Item $dest $old -ErrorAction SilentlyContinue
        }
        try {
            Move-Item $tmp $dest -Force
        } catch {
            Fail "cannot replace $dest (is jira running, or the directory read-only?). Close jira and its MCP client and retry, or use -InstallDir <dir>."
        }
        # Remains only while a still-running process maps the old image; the
        # next install retries.
        if ($old) { Remove-Item $old -Force -ErrorAction SilentlyContinue }
    } finally {
        Remove-Item $tmp, $sum -Force -ErrorAction SilentlyContinue
    }

    # Run it right away so a broken download surfaces here, not later.
    & (Join-Path $InstallDir 'jira.exe') --version

    # ---- PATH ----------------------------------------------------------------

    if (Add-ToUserPath $InstallDir) {
        Update-Environment
        Write-Host "Added $InstallDir to the user PATH."
    }
    Write-Host "Open a new terminal if this is a fresh install, then run: jira auth login"
}

# Leave no function definition behind in an interactive session either.
try {
    Install-Jira
} finally {
    Remove-Item function:Install-Jira -ErrorAction SilentlyContinue
}
