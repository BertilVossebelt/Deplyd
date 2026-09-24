<#
    Turning a value into JSON text deplyd controls the shape of.

    ConvertTo-Json is not usable for anything written to disk or read by another
    program: Windows PowerShell 5.1 indents each level to line up under the key it
    belongs to, which walks off the right of the screen, while 7 emits ordinary
    two-space JSON. Same command, two different files, on the two versions deplyd
    supports. This produces the same bytes on both.
#>

function ConvertTo-JsonString {
    param([string] $Value)

    $builder = New-Object System.Text.StringBuilder
    [void] $builder.Append('"')
    foreach ($character in $Value.ToCharArray()) {
        switch ($character) {
            '"'      { [void] $builder.Append('\"'); continue }
            '\'      { [void] $builder.Append('\\'); continue }
            ([char]8)  { [void] $builder.Append('\b'); continue }
            ([char]9)  { [void] $builder.Append('\t'); continue }
            ([char]10) { [void] $builder.Append('\n'); continue }
            ([char]12) { [void] $builder.Append('\f'); continue }
            ([char]13) { [void] $builder.Append('\r'); continue }
            default {
                if ([int] $character -lt 32) {
                    [void] $builder.Append('\u{0:x4}' -f [int] $character)
                } else {
                    [void] $builder.Append($character)
                }
            }
        }
    }
    [void] $builder.Append('"')
    return $builder.ToString()
}

function ConvertTo-JsonText {
    param($Value, [int] $Depth = 0)

    $indent = '  ' * $Depth
    $inner = '  ' * ($Depth + 1)

    if ($null -eq $Value) { return 'null' }
    if ($Value -is [bool]) { if ($Value) { return 'true' } else { return 'false' } }
    if ($Value -is [int] -or $Value -is [long] -or $Value -is [double] -or $Value -is [decimal]) {
        return ([string] $Value)
    }
    if ($Value -is [string]) { return (ConvertTo-JsonString -Value $Value) }

    # PSCustomObject, as ConvertFrom-Json hands back, alongside the ordered hashtables
    # the reports are built from.
    if ($Value -is [System.Management.Automation.PSCustomObject]) {
        $pairs = [ordered]@{}
        foreach ($property in $Value.PSObject.Properties) { $pairs[$property.Name] = $property.Value }
        $Value = $pairs
    }

    if ($Value -is [System.Collections.IDictionary]) {
        if ($Value.Count -eq 0) { return '{}' }
        $lines = @()
        foreach ($key in $Value.Keys) {
            $lines += $inner + (ConvertTo-JsonString -Value ([string] $key)) + ': ' +
                (ConvertTo-JsonText -Value $Value[$key] -Depth ($Depth + 1))
        }
        return "{`n" + ($lines -join ",`n") + "`n$indent}"
    }

    if ($Value -is [System.Collections.IEnumerable]) {
        $items = @($Value)
        if ($items.Count -eq 0) { return '[]' }
        $lines = @()
        foreach ($item in $items) {
            $lines += $inner + (ConvertTo-JsonText -Value $item -Depth ($Depth + 1))
        }
        return "[`n" + ($lines -join ",`n") + "`n$indent]"
    }

    return (ConvertTo-JsonString -Value ([string] $Value))
}
