-- Print the id of the first scene that holds a widget with the given uid.
-- Reads a GetScenes gRPC-web response on stdin; a tiny protobuf walker, so
-- the Deck needs no extra packages:
--   GetScenesResponse { repeated Scene scenes = 1; }   Scene { string id = 1; ... }
-- Usage: lua scene.lua <widget-uid> < response
local uid = arg[1]
local d = io.read("*a")

local function varint(s, i)
  local v, mul = 0, 1
  while true do
    local b = s:byte(i)
    if not b then return nil, i end
    i = i + 1
    v = v + (b % 128) * mul
    if b < 128 then return v, i end
    mul = mul * 128
  end
end

-- Walk the fields of message `s`; call on_bytes(field, payload) for each
-- length-delimited one. Returns early when on_bytes returns a value.
local function walk(s, on_bytes)
  local i = 1
  while i <= #s do
    local key
    key, i = varint(s, i)
    if not key then return end
    local field, wire = math.floor(key / 8), key % 8
    if wire == 0 then
      local _; _, i = varint(s, i)
    elseif wire == 1 then i = i + 8
    elseif wire == 5 then i = i + 4
    elseif wire == 2 then
      local len; len, i = varint(s, i)
      if not len then return end
      local r = on_bytes(field, s:sub(i, i + len - 1))
      if r then return r end
      i = i + len
    else return end
  end
end

if #d < 5 or d:byte(1) ~= 0 then os.exit(1) end
local len = ((d:byte(2) * 256 + d:byte(3)) * 256 + d:byte(4)) * 256 + d:byte(5)
local id = walk(d:sub(6, 5 + len), function(field, scene)
  if field == 1 and scene:find(uid, 1, true) then
    return walk(scene, function(f, v) if f == 1 then return v end end)
  end
end)
if not id then os.exit(1) end
print(id)
