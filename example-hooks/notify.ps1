# A deplyd hook: one desktop notification per event, in the Action Center.
#
#   deplyd hooks add .\example-hooks\notify.ps1
#   deplyd hooks test
#
# Deplyd hands a hook one JSON event on stdin and nothing else. Copy this and
# change it; it is meant to be a starting point, not a dependency.

$ErrorActionPreference = 'Stop'

$payload = [Console]::In.ReadToEnd()
if ([string]::IsNullOrWhiteSpace($payload)) { exit 0 }
$event = $payload | ConvertFrom-Json

# What the event is, said the way a person would say it.
switch ($event.kind) {
    'deploy.started'   { $headline = "Deploying $($event.label)" }
    'deploy.succeeded' { $headline = "$($event.label) is live" }
    'deploy.failed'    { $headline = "$($event.label) failed to deploy" }
    'change.live'      { $headline = "Your change is live on $($event.label)" }
    default            { $headline = "$($event.label): $($event.kind)" }
}

# WinRT rather than a balloon tip: this one stays in the Action Center, so a
# deploy that finished while you were elsewhere is still there when you look.
[void][Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime]
[void][Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom, ContentType = WindowsRuntime]

$template = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent(
    [Windows.UI.Notifications.ToastTemplateType]::ToastText02)

$texts = $template.GetElementsByTagName('text')
$texts.Item(0).AppendChild($template.CreateTextNode($headline)) | Out-Null
$texts.Item(1).AppendChild($template.CreateTextNode($event.title)) | Out-Null

# Borrowing PowerShell's own identity, which Windows already knows about.
# Without a registered sender nothing is shown at all.
#
# It does mean the toast says "Windows PowerShell" rather than Deplyd. Showing
# your own name means registering one - a key under HKCU on Windows, an app
# bundle on macOS - and that is more than an example hook should be doing to
# someone's machine. Register one of your own if you want it.
$appId = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe'

$toast = [Windows.UI.Notifications.ToastNotification]::new($template)
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($appId).Show($toast)
