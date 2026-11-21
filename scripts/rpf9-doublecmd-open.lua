local script = debug.getinfo(1, "S").source:sub(2)
local backend = script:match("^(.*[/])") .. "rpf9-wfx-backend.py"
local archive = DC.ExpandVar('%"0%p0')
local destination = "wfx://RPF9"

local function shell_quote(text)
    return "'" .. text:gsub("'", "'\\''") .. "'"
end

if archive:lower():sub(-4) == ".rpf" then
    local pipe = io.popen(shell_quote(backend) .. " register " .. shell_quote(archive), "r")
    if not pipe then
        DC.LogWrite("Could not start the RPF9 archive browser.", 2)
        return
    end
    local slot = pipe:read("*l")
    if not pipe:close() or not slot or not slot:match("^[%w_.-]+%-%-[0-9a-f]+$") then
        DC.LogWrite("Could not open this RPF9 archive.", 2)
        return
    end
    destination = destination .. "/" .. slot
end

DC.ExecuteCommand("cm_ChangeDir", destination)
