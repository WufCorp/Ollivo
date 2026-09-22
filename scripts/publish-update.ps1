<#
.SYNOPSIS
    Собирает Ollivo, подписывает обновление и кладёт его в S3 (Timeweb).

.DESCRIPTION
    Канал stable → updates/latest.json, beta → updates/latest-beta.json.
    Установщик всегда лежит рядом под своим именем, поэтому старые версии не затираются.

    Ключ подписи: ~\.tauri\ollivo.key (вне репозитория, без пароля).
    Доступ к S3: профиль AWS CLI, по умолчанию timeweb.

.EXAMPLE
    ./scripts/publish-update.ps1 -Notes "Чат со стримингом" -Channel beta
#>
param(
  [ValidateSet('stable', 'beta')][string]$Channel = 'stable',
  [string]$KeyCredential = 'ollivo-updater-key',
  [string]$Notes = '',
  [string]$Profile = 'timeweb',
  [string]$Bucket = 'prisma-prava',
  [string]$Prefix = 'ollivo/updates',
  [string]$Endpoint = 'https://s3.twcstorage.ru',
  [string]$KeyPath = "$env:USERPROFILE\.tauri\ollivo.key",
  # Только собрать, подписать и показать latest.json — в S3 ничего не отправлять.
  [switch]$DryRun,
  # Установщик уже собран — не пересобирать.
  [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
# Иначе кириллица в сообщениях превращается в кракозябры (консоль в OEM-кодировке).
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$root = Split-Path $PSScriptRoot -Parent

function Fail($text) { Write-Error $text; exit 1 }

# Пароль ключа подписи — в диспетчере учётных данных Windows (cmdkey /generic:ollivo-updater-key),
# рядом с ключом не лежит. Читаем через CredRead: своего способа у PowerShell нет.
function Get-KeyPassword($target) {
  if (-not ('Ollivo.Cred' -as [type])) {
    Add-Type -Namespace Ollivo -Name Cred -MemberDefinition @'
[DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
public static extern bool CredReadW(string target, uint type, uint flags, out IntPtr credential);
[DllImport("advapi32.dll")]
public static extern void CredFree(IntPtr buffer);
[StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
public struct CREDENTIAL {
  public uint Flags; public uint Type; public string TargetName; public string Comment;
  public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
  public uint CredentialBlobSize; public IntPtr CredentialBlob;
  public uint Persist; public uint AttributeCount; public IntPtr Attributes;
  public string TargetAlias; public string UserName;
}
public static string Read(string target) {
  IntPtr p;
  if (!CredReadW(target, 1, 0, out p)) return null;
  try {
    var c = (CREDENTIAL)Marshal.PtrToStructure(p, typeof(CREDENTIAL));
    return Marshal.PtrToStringUni(c.CredentialBlob, (int)(c.CredentialBlobSize / 2));
  } finally { CredFree(p); }
}
'@
  }
  [Ollivo.Cred]::Read($target)
}

# --- Версия: одна и та же в трёх местах ---
$conf = Get-Content "$root\src-tauri\tauri.conf.json" -Raw | ConvertFrom-Json
$version = $conf.version
$pkg = (Get-Content "$root\package.json" -Raw | ConvertFrom-Json).version
$cargo = (Select-String -Path "$root\src-tauri\Cargo.toml" -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
if ($pkg -ne $version -or $cargo -ne $version) {
  Fail "Версии разные: tauri.conf.json $version, package.json $pkg, Cargo.toml $cargo"
}
Write-Host "Версия $version, канал $Channel"

if (-not (Test-Path $KeyPath)) { Fail "Нет ключа подписи: $KeyPath" }

# --- Сборка ---
# Подписываем отдельной командой, а не переменными окружения при сборке:
# у ключа нет пароля, а пустую переменную окружения Windows не хранит, и tauri
# в конце сборки спрашивал бы пароль в консоли.
$keyPassword = Get-KeyPassword $KeyCredential
if (-not $keyPassword) { Fail "Нет пароля ключа в диспетчере учётных данных: $KeyCredential" }

if (-not $SkipBuild) {
  $env:TAURI_SIGNING_PRIVATE_KEY = Get-Content $KeyPath -Raw
  $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $keyPassword
  Push-Location $root
  try { npm run tauri build; if ($LASTEXITCODE -ne 0) { Fail 'Сборка не удалась' } }
  finally {
    Pop-Location
    $env:TAURI_SIGNING_PRIVATE_KEY = $null
    $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $null
  }
}

$setup = "$root\src-tauri\target\release\bundle\nsis\Ollivo_${version}_x64-setup.exe"
if (-not (Test-Path $setup)) { Fail "Нет установщика: $setup" }

# Подпись создаётся вместе со сборкой (bundle.createUpdaterArtifacts).
# С -SkipBuild подписываем отдельно: установщик мог быть собран без ключа.
if (-not (Test-Path "$setup.sig")) {
  npx tauri signer sign -f $KeyPath -p $keyPassword --app-version $version $setup
  if ($LASTEXITCODE -ne 0 -or -not (Test-Path "$setup.sig")) { Fail 'Не удалось подписать установщик' }
}

# --- Описание версии ---
$file = if ($Channel -eq 'beta') { 'latest-beta.json' } else { 'latest.json' }
$exeName = Split-Path $setup -Leaf
$manifest = [ordered]@{
  version   = $version
  notes     = $Notes
  pub_date  = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
  platforms = [ordered]@{
    'windows-x86_64' = [ordered]@{
      signature = (Get-Content "$setup.sig" -Raw).Trim()
      url       = "$Endpoint/$Bucket/$Prefix/$exeName"
    }
  }
}
$json = $manifest | ConvertTo-Json -Depth 5
$jsonPath = Join-Path ([System.IO.Path]::GetTempPath()) $file
# Без BOM: Set-Content -Encoding utf8 в Windows PowerShell добавляет его,
# и программа не может разобрать такой JSON («не получилось связаться с сервером»).
[System.IO.File]::WriteAllText($jsonPath, $json, (New-Object System.Text.UTF8Encoding $false))

Write-Host "`n${file}:`n$json`n"
if ($DryRun) { Write-Host 'Проверка без отправки: в S3 ничего не загружено.'; exit 0 }

# --- Загрузка: сначала установщик, потом описание ---
# Порядок важен: пока нет описания, обновление никому не предлагается,
# а описание без установщика вело бы на несуществующий файл.
$s3 = @('--profile', $Profile, '--endpoint-url', $Endpoint)
aws @s3 s3 cp $setup "s3://$Bucket/$Prefix/$exeName" --acl public-read
if ($LASTEXITCODE -ne 0) { Fail 'Не удалось загрузить установщик' }
aws @s3 s3 cp $jsonPath "s3://$Bucket/$Prefix/$file" --acl public-read --content-type application/json --cache-control 'no-cache'
if ($LASTEXITCODE -ne 0) { Fail 'Не удалось загрузить описание версии' }

Write-Host "`nГотово:"
Write-Host "  $Endpoint/$Bucket/$Prefix/$file"
Write-Host "  $Endpoint/$Bucket/$Prefix/$exeName"
