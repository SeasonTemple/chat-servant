# Package the SvelteKit static build in ui/build into a GitHub Pages bundle.
# Supports root ('/') and repository subpath (e.g. '/chat-servant') deployments.

param(
  [string]$BasePath = '/chat-servant',
  [string]$OutDir = 'target/pages',
  [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $repoRoot

$utf8NoBom = New-Object System.Text.UTF8Encoding($false)

function Write-Utf8Lf([string]$Path, [string]$Content) {
  $dir = Split-Path -Parent $Path
  if ($dir -and -not (Test-Path $dir)) {
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
  }
  $normalized = $Content -replace "`r`n", "`n" -replace "`r", "`n"
  [System.IO.File]::WriteAllText($Path, $normalized, $utf8NoBom)
}

if (-not $SkipBuild) {
  Write-Output 'Building static UI in ui/ ...'
  Push-Location (Join-Path $repoRoot 'ui')
  try {
    & pnpm build
    if ($LASTEXITCODE -ne 0) {
      throw "pnpm build failed with exit code $LASTEXITCODE"
    }
  } finally {
    Pop-Location
  }
}

$buildDir = Join-Path $repoRoot 'ui/build'
if (-not (Test-Path $buildDir)) {
  throw "Static build directory not found: $buildDir"
}

$outAbs = if ([System.IO.Path]::IsPathRooted($OutDir)) {
  $OutDir
} else {
  Join-Path $repoRoot $OutDir
}

if (Test-Path $outAbs) {
  Remove-Item -Path $outAbs -Recurse -Force
}
New-Item -ItemType Directory -Path $outAbs -Force | Out-Null

Get-ChildItem -Path $buildDir -Force | ForEach-Object {
  Copy-Item -Path $_.FullName -Destination $outAbs -Recurse -Force
}

# Prevent GitHub Pages from ignoring the _app directory.
Write-Utf8Lf (Join-Path $outAbs '.nojekyll') ''

$indexHtml = Join-Path $outAbs 'index.html'
$promoHtml = Join-Path $outAbs 'promo.html'
$panelHtml = Join-Path $outAbs 'panel.html'
$appRootHtml = Join-Path $outAbs 'app-root.html'
$panelSubHtml = Join-Path $outAbs 'panel/index.html'
$promoSubHtml = Join-Path $outAbs 'promo/index.html'

if (-not (Test-Path $panelHtml) -and (Test-Path $indexHtml)) {
  Copy-Item -Path $indexHtml -Destination $panelHtml -Force
}
if ((Test-Path $panelHtml) -and -not (Test-Path $panelSubHtml)) {
  $panelContent = [System.IO.File]::ReadAllText($panelHtml, $utf8NoBom)
  $panelSubContent = $panelContent.Replace('"./_app/', '"../_app/').Replace('''./_app/', '''../_app/').Replace('new URL(".", location)', 'new URL("..", location)')
  Write-Utf8Lf $panelSubHtml $panelSubContent
}

if (Test-Path $promoHtml) {
  if (Test-Path $indexHtml) {
    Copy-Item -Path $indexHtml -Destination $appRootHtml -Force
  }
  $promoContent = [System.IO.File]::ReadAllText($promoHtml, $utf8NoBom)
  Write-Utf8Lf $indexHtml $promoContent
  $promoSubContent = $promoContent.Replace('"./_app/', '"../_app/').Replace('''./_app/', '''../_app/').Replace('new URL(".", location)', 'new URL("..", location)')
  Write-Utf8Lf $promoSubHtml $promoSubContent

  # Point the root client-side route entry to /promo so SPA hydration at '/' renders the promo page.
  $entryDir = Join-Path $outAbs '_app/immutable/entry'
  if (Test-Path $entryDir) {
    Get-ChildItem -Path $entryDir -Filter 'app.*.js' -File | ForEach-Object {
      $appJs = [System.IO.File]::ReadAllText($_.FullName, $utf8NoBom)
      if ($appJs -match '"/promo":(\[[0-9,\[\]]+\])') {
        $promoNode = $Matches[1]
        $appJs = [regex]::Replace($appJs, '"\/\(panel\)":\[[0-9,\[\]]+\]', ('"/(panel)":' + $promoNode))
        Write-Utf8Lf $_.FullName $appJs
      }
    }
  }
}

$normBase = $BasePath.Trim()
if ($normBase -eq '/') {
  $normBase = ''
} elseif ($normBase.Length -gt 0) {
  if (-not $normBase.StartsWith('/')) {
    $normBase = '/' + $normBase
  }
  $normBase = $normBase.TrimEnd('/')
}

$textFiles = Get-ChildItem -Path $outAbs -Recurse -File | Where-Object {
  $_.Extension -in @('.html', '.js', '.css')
}

if ($normBase.Length -gt 0) {
  foreach ($file in $textFiles) {
    $content = [System.IO.File]::ReadAllText($file.FullName, $utf8NoBom)
    $updated = $content

    if ($file.Extension -eq '.html') {
      $updated = $updated.Replace('"./_app/', ('"' + $normBase + '/_app/'))
      $updated = $updated.Replace('''./_app/', ('''' + $normBase + '/_app/'))
      $updated = $updated.Replace('"../_app/', ('"' + $normBase + '/_app/'))
      $updated = $updated.Replace('''../_app/', ('''' + $normBase + '/_app/'))
      $updated = $updated.Replace('base: ""', ('base: "' + $normBase + '"'))
      $updated = $updated.Replace('base: new URL(".", location).pathname.slice(0, -1)', ('base: "' + $normBase + '"'))
      $updated = $updated.Replace('base: new URL("..", location).pathname.slice(0, -1)', ('base: "' + $normBase + '"'))
    }

    $escBase = [regex]::Escape($normBase)
    $updated = [regex]::Replace($updated, ('(["''`])(?!' + $escBase + '/)/_app/'), ('$1' + $normBase + '/_app/'))
    $updated = [regex]::Replace($updated, ('(["''`(])(?!' + $escBase + '/)/probe-sheet\.svg'), ('$1' + $normBase + '/probe-sheet.svg'))
    $updated = [regex]::Replace($updated, ('(["''`(])(?!' + $escBase + '/)/probe-sheet\.png'), ('$1' + $normBase + '/probe-sheet.png'))
    $updated = [regex]::Replace($updated, ('(["''`(])(?!' + $escBase + '/)/strip\.png'), ('$1' + $normBase + '/strip.png'))
    $updated = [regex]::Replace($updated, ('(["''`(])(?!' + $escBase + '/)/fonts/'), ('$1' + $normBase + '/fonts/'))
    $updated = [regex]::Replace($updated, ('(["''`])(?!' + $escBase + '/)/demo/'), ('$1' + $normBase + '/demo/'))
    $updated = [regex]::Replace($updated, ('(["''`])(?!' + $escBase + '/)/panel(?=[/?#"''`\s>])'), ('$1' + $normBase + '/panel'))

    if ($updated -ne $content -or $content.Contains("`r")) {
      Write-Utf8Lf $file.FullName $updated
    }
  }
} else {
  foreach ($file in $textFiles) {
    $content = [System.IO.File]::ReadAllText($file.FullName, $utf8NoBom)
    if ($content.Contains("`r")) {
      Write-Utf8Lf $file.FullName $content
    }
  }
}

Write-Output ("Pages bundle ready at {0} (BasePath='{1}')" -f $outAbs, $BasePath)
