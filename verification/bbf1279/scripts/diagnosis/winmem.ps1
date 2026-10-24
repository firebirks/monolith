param($out)
for ($i = 0; $i -lt 240; $i++) {
  $os = Get-CimInstance Win32_OperatingSystem
  $pf = (Get-Counter '\Memory\Pages/sec' -ErrorAction SilentlyContinue).CounterSamples[0].CookedValue
  "{0} freeGB={1:N2} pagesPerSec={2:N0}" -f (Get-Date -Format s), ($os.FreePhysicalMemory/1MB), $pf | Out-File -Append -FilePath $out
  Start-Sleep -Seconds 8
}
