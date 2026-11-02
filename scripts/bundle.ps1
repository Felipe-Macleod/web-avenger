# Monta uma pasta autocontida (dist\web-avenger) com o executável + runtime do CEF.
# Equivalente Windows do scripts/bundle.sh.
$ErrorActionPreference = 'Stop'

Set-Location (Join-Path $PSScriptRoot '..')
$BuildProfile = if ($env:PROFILE) { $env:PROFILE } else { 'release' }
# O `cargo install` coloca o cargo-about aqui, que nem sempre está no PATH.
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
if (-not (Get-Command cargo-about -ErrorAction SilentlyContinue)) {
    throw 'Falta o cargo-about (licenças de terceiros): cargo install --locked cargo-about --features cli'
}
# O cargo escreve o progresso no stderr; com 'Stop', o PowerShell 5.1 trataria isso
# como erro quando a saída é redirecionada. O resultado vem do código de saída.
$ErrorActionPreference = 'Continue'
cargo build --profile $BuildProfile
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$ErrorActionPreference = 'Stop'

$TargetDir = "target\$BuildProfile"
$Dist = 'dist\web-avenger'

# Diretório do CEF: dentro de $env:CEF_PATH (o cef-dll-sys baixa para <CEF_PATH>\<versão>\<pasta>)
# ou o baixado pelo build do cef-dll-sys em target\. Em ambos, o libcef.dll mais recente.
$lib = $null
if ($env:CEF_PATH) {
    $lib = Get-ChildItem $env:CEF_PATH -Recurse -Filter libcef.dll -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
}
if (-not $lib) {
    $lib = Get-ChildItem "$TargetDir\build\cef-dll-sys-*\out\*" -Recurse -Filter libcef.dll |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
}
if (-not $lib) { throw "libcef.dll não encontrado em $TargetDir\build" }
$CefRoot = $lib.DirectoryName
# Na distribuição oficial do CEF as DLLs ficam em Release\ e os recursos em Resources\;
# o cef-dll-sys normalmente já entrega tudo numa pasta só.
if (Test-Path "$CefRoot\..\Resources") { $CefRoot = (Resolve-Path "$CefRoot\..").Path }
$CefDirs = @('Release', 'Resources') | ForEach-Object { Join-Path $CefRoot $_ } | Where-Object { Test-Path $_ }
if (-not $CefDirs) { $CefDirs = @($CefRoot) }
Write-Host "Usando CEF de: $($CefDirs -join ', ')"

if (Test-Path $Dist) { Remove-Item $Dist -Recurse -Force }
New-Item -ItemType Directory -Force $Dist | Out-Null

# Copia DLLs, .pak, icudtl.dat, snapshots do V8, locales\ etc. (ignora headers e artefatos de build).
$Skip = @('include', 'cmake', 'build', 'libcef_dll', 'Release', 'Resources', 'archive.json')
foreach ($dir in $CefDirs) {
    Get-ChildItem $dir | Where-Object {
        $_.Name -notin $Skip -and $_.Name -notlike 'CMakeLists*' -and
        $_.Extension -notin @('.txt', '.cmake', '.lib', '.pdb')
    } | Copy-Item -Destination $Dist -Recurse -Force
}
Copy-Item "$TargetDir\web-avenger.exe" $Dist

# Licenças: a do app, a do CEF (o CREDITS.html do CEF só cobre o Chromium) e a dos crates.
Copy-Item LICENSE "$Dist\LICENSE.txt"
Copy-Item licenses\CEF.txt "$Dist\CEF-LICENSE.txt"
$ErrorActionPreference = 'Continue'
cargo about generate --fail -c licenses\about.toml licenses\about.hbs -o "$Dist\THIRD_PARTY_LICENSES.html"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$ErrorActionPreference = 'Stop'

$Size = (Get-ChildItem $Dist -Recurse -File | Measure-Object Length -Sum).Sum / 1MB
Write-Host ("Pronto: $Dist\web-avenger.exe ({0:N0} MB)" -f $Size)
