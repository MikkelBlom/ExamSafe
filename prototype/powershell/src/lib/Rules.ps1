# Rules: load the rule files and classify inventory items against them.
# Everything here is pure (no system changes) so it can be unit tested with fake items.

$script:VALID_ACTIONS = @('stop', 'review', 'allow')
$script:VALID_TYPES = @('process', 'service', 'task', 'startup', 'port')

function Import-EsRuleSet {
    param(
        [Parameter(Mandatory = $true)] [string] $DefaultPath,
        [string] $ProfilePath
    )
    $defaults = Get-Content -Raw -LiteralPath $DefaultPath | ConvertFrom-Json
    $userProfile = $null
    if ($ProfilePath -and (Test-Path -LiteralPath $ProfilePath)) {
        $userProfile = Get-Content -Raw -LiteralPath $ProfilePath | ConvertFrom-Json
    }
    ConvertTo-EsRuleSet -Defaults $defaults -UserProfile $userProfile
}

function ConvertTo-EsRuleSet {
    param(
        [Parameter(Mandatory = $true)] $Defaults,
        $UserProfile
    )
    $categories = @{}
    foreach ($prop in $Defaults.categories.PSObject.Properties) {
        Assert-EsAction -Action $prop.Value.action -Context "category '$($prop.Name)'"
        $categories[$prop.Name] = [pscustomobject]@{
            Name        = $prop.Name
            Label       = $prop.Value.label
            Action      = $prop.Value.action
            Description = $prop.Value.description
        }
    }

    $disabledRules = @()
    $rules = New-Object System.Collections.Generic.List[object]
    if ($UserProfile) {
        if ($UserProfile.categoryActions) {
            foreach ($prop in $UserProfile.categoryActions.PSObject.Properties) {
                if (-not $categories.ContainsKey($prop.Name)) { throw "my-profile.json: unknown category '$($prop.Name)' in categoryActions." }
                Assert-EsAction -Action $prop.Value -Context "categoryActions.$($prop.Name)"
                $categories[$prop.Name].Action = $prop.Value
            }
        }
        if ($UserProfile.disabledRules) { $disabledRules = @($UserProfile.disabledRules) }
        # Profile rules go first so personal rules win over the generic ones.
        foreach ($rule in @($UserProfile.rules)) {
            if ($rule) { $rules.Add((New-EsRule -Raw $rule -Source 'profile' -Categories $categories)) }
        }
    }
    foreach ($rule in @($Defaults.rules)) {
        if ($rule -and ($disabledRules -notcontains $rule.id)) {
            $rules.Add((New-EsRule -Raw $rule -Source 'default' -Categories $categories))
        }
    }
    [pscustomobject]@{ Categories = $categories; Rules = $rules.ToArray() }
}

function Assert-EsAction {
    param($Action, [string] $Context)
    if ($script:VALID_ACTIONS -notcontains $Action) {
        throw "Invalid action '$Action' in $Context. Use one of: $($script:VALID_ACTIONS -join ', ')."
    }
}

function New-EsRule {
    param($Raw, [string] $Source, [hashtable] $Categories)
    if (-not $Raw.id) { throw "A $Source rule is missing 'id'." }
    if (-not $Categories.ContainsKey([string]$Raw.category)) { throw "Rule '$($Raw.id)': unknown category '$($Raw.category)'." }
    if ($Raw.action) { Assert-EsAction -Action $Raw.action -Context "rule '$($Raw.id)'" }
    $matchEntries = @($Raw.match | Where-Object { $_ })
    if ($matchEntries.Count -eq 0) { throw "Rule '$($Raw.id)' has no match entries." }
    foreach ($m in $matchEntries) {
        if ($script:VALID_TYPES -notcontains $m.type) { throw "Rule '$($Raw.id)': unknown match type '$($m.type)'." }
        if (-not $m.name -and -not $m.path -and -not $m.port) { throw "Rule '$($Raw.id)': a match entry needs name, path or port." }
    }
    [pscustomobject]@{
        Id       = [string]$Raw.id
        Label    = if ($Raw.label) { [string]$Raw.label } else { [string]$Raw.id }
        Category = [string]$Raw.category
        Action   = if ($Raw.action) { [string]$Raw.action } else { $null }
        Match    = $matchEntries
        Source   = $Source
    }
}

function Test-EsMatch {
    param($Item, $Match)
    if ($Match.type -ne $Item.Type) { return $false }
    if ($Match.name -and -not ([string]$Item.Name -like [string]$Match.name)) { return $false }
    if ($Match.path -and -not ([string]$Item.Path -like [string]$Match.path)) { return $false }
    if ($Match.port -and ([int]$Item.Port -ne [int]$Match.port)) { return $false }
    return $true
}

function Find-EsRule {
    # First matching rule wins; returns @{ Rule; Match } or $null.
    param($Item, $RuleSet)
    foreach ($rule in $RuleSet.Rules) {
        foreach ($m in $rule.Match) {
            if (Test-EsMatch -Item $Item -Match $m) { return @{ Rule = $rule; Match = $m } }
        }
    }
    return $null
}

function Get-EsClassification {
    # Returns copies of the items with RuleId/Category/Action/Relaunch filled in.
    # Items that match no rule get Action 'unclassified'. Port items without their own rule
    # inherit the classification of the process that owns the port.
    param(
        [Parameter(Mandatory = $true)] [AllowEmptyCollection()] [object[]] $Items,
        [Parameter(Mandatory = $true)] $RuleSet
    )
    $result = New-Object System.Collections.Generic.List[object]
    $processByPid = @{}
    foreach ($item in $Items) {
        $copy = $item.PSObject.Copy()
        $found = Find-EsRule -Item $copy -RuleSet $RuleSet
        if ($found) {
            Set-EsClassification -Item $copy -Rule $found.Rule -RuleSet $RuleSet -Relaunch ([bool]$found.Match.relaunch)
        } else {
            Set-EsClassification -Item $copy -Rule $null -RuleSet $RuleSet -Relaunch $false
        }
        if ($copy.Type -eq 'process') { foreach ($procId in @($copy.Pids)) { $processByPid[[int]$procId] = $copy } }
        $result.Add($copy)
    }
    foreach ($copy in $result) {
        if ($copy.Type -eq 'port' -and -not $copy.RuleId -and $copy.OwnerPid -and $processByPid.ContainsKey([int]$copy.OwnerPid)) {
            $owner = $processByPid[[int]$copy.OwnerPid]
            if ($owner.RuleId) {
                $copy.RuleId = $owner.RuleId
                $copy.RuleLabel = $owner.RuleLabel
                $copy.Category = $owner.Category
                $copy.CategoryLabel = $owner.CategoryLabel
                $copy.Action = $owner.Action
            }
        }
    }
    $result.ToArray()
}

function Set-EsClassification {
    param($Item, $Rule, $RuleSet, [bool] $Relaunch)
    if ($Rule) {
        $category = $RuleSet.Categories[$Rule.Category]
        $action = if ($Rule.Action) { $Rule.Action } else { $category.Action }
        $values = @{ RuleId = $Rule.Id; RuleLabel = $Rule.Label; Category = $Rule.Category; CategoryLabel = $category.Label; Action = $action; Relaunch = $Relaunch }
    } else {
        $values = @{ RuleId = $null; RuleLabel = $null; Category = $null; CategoryLabel = 'Unclassified'; Action = 'unclassified'; Relaunch = $false }
    }
    foreach ($key in $values.Keys) { $Item | Add-Member -NotePropertyName $key -NotePropertyValue $values[$key] -Force }
}

function Select-EsRelevantItems {
    # Hide Windows' own plumbing unless a rule explicitly talks about it.
    param([AllowEmptyCollection()] [object[]] $Items)
    @($Items | Where-Object { $_.RuleId -or -not $_.IsSystem })
}

function New-EsUserRule {
    # Builds a profile rule that matches one inventory item exactly (wildcards escaped).
    param(
        [Parameter(Mandatory = $true)] $Item,
        [Parameter(Mandatory = $true)] [ValidateSet('stop', 'allow', 'review')] [string] $Action
    )
    $safeName = ($Item.Name -replace '[^A-Za-z0-9]+', '-').Trim('-').ToLowerInvariant()
    $match = [ordered]@{ type = $Item.Type }
    if ($Item.Type -eq 'port') { $match.port = [int]$Item.Port }
    else { $match.name = [System.Management.Automation.WildcardPattern]::Escape([string]$Item.Name) }
    if ($Item.Type -eq 'process' -and $Action -eq 'stop' -and $Item.Path) { $match.relaunch = $true }
    [pscustomobject][ordered]@{
        id       = "user-$($Item.Type)-$safeName"
        label    = [string]$Item.Name
        category = 'custom'
        action   = $Action
        match    = @([pscustomobject]$match)
    }
}

function Add-EsProfileRule {
    # Adds (or replaces, by id) a rule in my-profile.json.
    param(
        [Parameter(Mandatory = $true)] [string] $ProfilePath,
        [Parameter(Mandatory = $true)] $Rule
    )
    if (Test-Path -LiteralPath $ProfilePath) {
        $data = Get-Content -Raw -LiteralPath $ProfilePath | ConvertFrom-Json
    } else {
        $data = [pscustomobject]@{ categoryActions = [pscustomobject]@{}; disabledRules = @(); rules = @() }
    }
    $kept = @(@($data.rules) | Where-Object { $_ -and $_.id -ne $Rule.id })
    # New rules go first: the most recent decision should win.
    $data.rules = @($Rule) + $kept
    $json = $data | ConvertTo-Json -Depth 10
    Write-EsFileAtomic -Path $ProfilePath -Content $json
}

function Write-EsFileAtomic {
    param([string] $Path, [string] $Content)
    $dir = Split-Path -Parent $Path
    if ($dir -and -not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }
    $tmp = "$Path.tmp"
    [System.IO.File]::WriteAllText($tmp, $Content, (New-Object System.Text.UTF8Encoding($false)))
    Move-Item -LiteralPath $tmp -Destination $Path -Force
}
