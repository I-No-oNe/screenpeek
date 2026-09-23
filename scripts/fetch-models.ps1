# Download extra Tesseract languages for screenpeek on Windows.
# usage: fetch-models.ps1 [heb ara ...] [-All] [-None]
param([string[]]$Languages = @(), [switch]$All, [switch]$None)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

# The same places screenpeek reads on Windows.
$tessdata = if ($env:TESSDATA_PREFIX) { $env:TESSDATA_PREFIX } else { Join-Path $env:APPDATA 'tessdata' }
$config = Join-Path $env:APPDATA 'screenpeek\languages'
$url = 'https://github.com/tesseract-ocr/tessdata_fast/raw/main'
$offered = [ordered]@{
    ara = 'Arabic'; chi_sim = 'Chinese, simplified'; deu = 'German'; fra = 'French'
    heb = 'Hebrew'; jpn = 'Japanese'; rus = 'Russian'; spa = 'Spanish'
    por = 'Portuguese'; ita = 'Italian'; nld = 'Dutch'; pol = 'Polish'
    tur = 'Turkish'; ukr = 'Ukrainian'; kor = 'Korean'; vie = 'Vietnamese'
}
$codes = @($offered.Keys)

$asked = $false
if ($All) {
    $Languages = $codes
} elseif (-not $None -and $Languages.Count -eq 0 -and -not $env:CI -and -not [Console]::IsInputRedirected) {
    # Ask only when someone is at the terminal to answer.
    $asked = $true
    Write-Host 'screenpeek reads English on its own.'
    Write-Host 'Pick extra languages to read (Hebrew, Arabic, Chinese...), or none.'
    Write-Host ''
    for ($i = 0; $i -lt $codes.Count; $i++) {
        $mark = if (Test-Path (Join-Path $tessdata "$($codes[$i]).traineddata")) { '* ' } else { '  ' }
        Write-Host ('{0}{1,2}) {2,-8} {3}' -f $mark, ($i + 1), $codes[$i], $offered[$codes[$i]])
    }
    Write-Host ''
    Write-Host '  * already installed. Enter numbers or codes, space separated.'
    $reply = Read-Host '  Enter for none, all for every one listed. languages'
    $Languages = @(foreach ($word in ($reply -split '\s+' | Where-Object { $_ })) {
        if ($word -eq 'all') { $codes } elseif ($word -match '^\d+$') { $codes[[int]$word - 1] } else { $word }
    })
}

if ($Languages.Count -eq 0) {
    # Forget an earlier choice only when someone chose none just now.
    if ($None -or $asked) { Remove-Item $config -ErrorAction SilentlyContinue }
    Write-Host 'no extra languages.'
    return
}

New-Item -ItemType Directory -Force $tessdata | Out-Null
foreach ($lang in @($Languages) + 'eng') {
    if ($lang -notmatch '^[A-Za-z0-9_]+$') { throw "invalid language code: $lang" }
    $file = Join-Path $tessdata "$lang.traineddata"
    if ((Test-Path $file) -and (Get-Item $file).Length -gt 0) { Write-Host "have    $lang"; continue }
    Write-Host "fetch   $lang"
    # Download beside the file, so an interrupted download leaves nothing behind.
    Invoke-WebRequest "$url/$lang.traineddata" -OutFile "$file.partial" -UseBasicParsing
    Move-Item -Force "$file.partial" $file
}

New-Item -ItemType Directory -Force (Split-Path $config) | Out-Null
Set-Content -Path $config -Value ($Languages -join ' ') -Encoding ascii
Write-Host ''
Write-Host "screenpeek will read: English $($Languages -join ' ')"
$tesseract = (Get-Command tesseract -ErrorAction SilentlyContinue) -or
    (Test-Path (Join-Path $env:ProgramFiles 'Tesseract-OCR\tesseract.exe'))
if (-not $tesseract) {
    Write-Host 'install Tesseract to use them: winget install UB-Mannheim.TesseractOCR'
}
