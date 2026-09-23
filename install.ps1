# Installs the latest screenpeek release for the current user:
#   irm https://raw.githubusercontent.com/I-No-oNe/screenpeek/main/install.ps1 | iex
# SCREENPEEK_PREFIX picks another folder than %LOCALAPPDATA%\screenpeek\bin.
# Run in its own scope, so the settings below stay out of the caller's session.
& {
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

    $repo = 'I-No-oNe/screenpeek'
    $prefix = if ($env:SCREENPEEK_PREFIX) { $env:SCREENPEEK_PREFIX } else { Join-Path $env:LOCALAPPDATA 'screenpeek\bin' }
    # ARM64 Windows also runs the x64 build, for releases that have no ARM64 one.
    $archives = @('screenpeek-x86_64-windows.zip')
    if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { $archives = @('screenpeek-aarch64-windows.zip') + $archives }

    # The newest release with a build for this machine; alphas count too.
    # Assigned first: Windows PowerShell 5.1 passes a JSON array on as one object.
    # A token, when set, lifts the shared rate limit of unauthenticated calls.
    $auth = @{}
    if ($env:GITHUB_TOKEN) { $auth.Authorization = "Bearer $env:GITHUB_TOKEN" }
    $releases = Invoke-RestMethod "https://api.github.com/repos/$repo/releases?per_page=20" -Headers $auth
    $release = $releases | Where-Object { $_.assets.name -contains $archives[-1] } | Select-Object -First 1
    if (-not $release) { throw "no release has a Windows build yet" }
    $archive = $archives | Where-Object { $release.assets.name -contains $_ } | Select-Object -First 1
    $tag = $release.tag_name
    $download = "https://github.com/$repo/releases/download/$tag"

    $tmp = Join-Path ([IO.Path]::GetTempPath()) "screenpeek-$([guid]::NewGuid())"
    New-Item -ItemType Directory $tmp | Out-Null
    try {
        $zip = Join-Path $tmp $archive
        Invoke-WebRequest "$download/$archive" -OutFile $zip -UseBasicParsing
        # Releases list a checksum beside each download; older ones have none.
        if ($release.assets.name -contains "$archive.sha256") {
            Invoke-WebRequest "$download/$archive.sha256" -OutFile "$zip.sha256" -UseBasicParsing
            $expected = ((Get-Content "$zip.sha256" -Raw).Trim() -split '\s+')[0]
            if ((Get-FileHash $zip -Algorithm SHA256).Hash -ne $expected) { throw "$archive does not match its checksum" }
            Write-Host "checksum ok"
        }
        New-Item -ItemType Directory -Force $prefix | Out-Null
        Expand-Archive $zip -DestinationPath $prefix -Force
    } finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($userPath -split ';') -notcontains $prefix) {
        [Environment]::SetEnvironmentVariable('Path', (@($userPath, $prefix) | Where-Object { $_ }) -join ';', 'User')
        Write-Host "added $prefix to your PATH; open a new terminal to use it there"
    }
    if (($env:Path -split ';') -notcontains $prefix) { $env:Path = "$env:Path;$prefix" }
    Write-Host "installed $tag to $prefix\screenpeek.exe"
    & (Join-Path $prefix 'screenpeek.exe') --version

    # The rest asks questions, so only when someone is at the terminal to answer.
    if ($env:CI -or [Console]::IsInputRedirected) { return }

    $agents = @{ claude = Join-Path $HOME '.claude\skills'; codex = Join-Path $HOME '.agents\skills' }
    $found = @($agents.Keys | Where-Object { Get-Command $_ -ErrorAction SilentlyContinue })
    if ($found.Count -gt 0 -and (Read-Host "Install the screenpeek skill for $($found -join ' and ')? [Y/n]") -notmatch '^[nN]') {
        $files = Invoke-RestMethod "https://api.github.com/repos/$repo/contents/skill/screenpeek?ref=$tag"
        foreach ($agent in $found) {
            $dir = Join-Path $agents[$agent] 'screenpeek'
            Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
            New-Item -ItemType Directory -Force $dir | Out-Null
            foreach ($file in $files) { Invoke-WebRequest $file.download_url -OutFile (Join-Path $dir $file.name) -UseBasicParsing }
            Write-Host "${agent}: $dir"
        }
        Write-Host 'Start a new agent session to load the skill.'
    }

    $fetch = Join-Path ([IO.Path]::GetTempPath()) 'screenpeek-fetch-models.ps1'
    Invoke-WebRequest "https://raw.githubusercontent.com/$repo/main/scripts/fetch-models.ps1" -OutFile $fetch -UseBasicParsing
    try { & $fetch } finally { Remove-Item $fetch -ErrorAction SilentlyContinue }
}
