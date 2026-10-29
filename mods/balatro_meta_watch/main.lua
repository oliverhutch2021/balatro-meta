-- Balatro Meta Watch: a panel in the corner of the screen showing what
-- `make watch` is doing. The watch script writes a short text file next to
-- the game's saves (balatro_meta_watch.txt); this mod reads it twice a
-- second and draws it with LOVE's own graphics calls, on top of the game.
-- It changes nothing in play. Written for the Balatro Meta project.

local FILE = "balatro_meta_watch.txt"
local MAX_LINES = 14
local lines = {}
local last_read = -1

local function read_status()
    local ok, contents = pcall(love.filesystem.read, FILE)
    lines = {}
    if not ok or type(contents) ~= "string" then
        return
    end
    for line in contents:gmatch("[^\r\n]+") do
        lines[#lines + 1] = line
        if #lines >= MAX_LINES then
            break
        end
    end
end

local function draw_panel()
    local now = love.timer.getTime()
    if now - last_read > 0.5 then
        last_read = now
        read_status()
    end
    if #lines == 0 then
        return
    end
    local font = love.graphics.getFont()
    local lh = font:getHeight()
    local w = 0
    for _, l in ipairs(lines) do
        w = math.max(w, font:getWidth(l))
    end
    local x, y = 14, 14
    love.graphics.push("all")
    love.graphics.setCanvas()
    love.graphics.origin()
    love.graphics.setShader()
    love.graphics.setColor(0, 0, 0, 0.75)
    love.graphics.rectangle("fill", x - 8, y - 6, w + 16, lh * #lines + 12, 6, 6)
    for i, l in ipairs(lines) do
        if l:find("MISMATCH", 1, true) then
            love.graphics.setColor(1, 0.45, 0.4, 1)
        else
            love.graphics.setColor(1, 1, 1, 1)
        end
        love.graphics.print(l, x, y + (i - 1) * lh)
    end
    love.graphics.pop()
end

local game_draw = love.draw
love.draw = function(...)
    if game_draw then
        game_draw(...)
    end
    pcall(draw_panel)
end
