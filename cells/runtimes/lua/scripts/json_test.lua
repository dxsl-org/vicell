-- json_test.lua — integration test for rxi/json.lua on ViCell
-- Run: lua /tmp/json_test.lua
-- Exercises: require (VFS-backed), pcall, string.*, table.*, VFS write

local json = require("json")

local passed = 0
local failed = 0

local function check(name, fn)
  local ok, err = pcall(fn)
  if ok then
    io.write("  PASS: " .. name .. "\n")
    passed = passed + 1
  else
    io.write("  FAIL: " .. name .. " — " .. tostring(err) .. "\n")
    failed = failed + 1
  end
end

io.write("=== json_test on ViCell Lua 5.4 ===\n")

-- 1. Basic object decode
check("decode object", function()
  local t = json.decode('{"os":"ViCell","version":2,"boot":true}')
  assert(t.os == "ViCell", "os=" .. tostring(t.os))
  assert(t.version == 2,   "version=" .. tostring(t.version))
  assert(t.boot == true,   "boot=" .. tostring(t.boot))
end)

-- 2. Array decode
check("decode array", function()
  local a = json.decode('[10, 20, 30]')
  assert(#a == 3,         "len=" .. #a)
  assert(a[1] == 10,      "a[1]=" .. a[1])
  assert(a[3] == 30,      "a[3]=" .. a[3])
end)

-- 3. Null decodes to nil
check("decode null", function()
  local t = json.decode('{"x":null}')
  assert(t.x == nil, "null should be nil")
end)

-- 4. Nested objects
check("decode nested", function()
  local t = json.decode('{"a":{"b":{"c":42}}}')
  assert(t.a.b.c == 42, "nested=" .. tostring(t.a.b.c))
end)

-- 5. Array of objects
check("decode array of objects", function()
  local t = json.decode('[{"id":1},{"id":2},{"id":3}]')
  assert(t[2].id == 2, "t[2].id=" .. tostring(t[2].id))
end)

-- 6. Encode → decode roundtrip
check("roundtrip", function()
  local orig = { name = "lua", major = 5, minor = 4, stable = true }
  local s = json.encode(orig)
  local back = json.decode(s)
  assert(back.name   == "lua", "name=" .. tostring(back.name))
  assert(back.major  == 5,     "major=" .. tostring(back.major))
  assert(back.stable == true,  "stable=" .. tostring(back.stable))
end)

-- 7. Encode array
check("encode array", function()
  local s = json.encode({1, 2, 3})
  assert(s == "[1,2,3]", "got: " .. s)
end)

-- 8. pcall catches invalid JSON
check("pcall on bad JSON", function()
  local ok, _ = pcall(json.decode, "{invalid}")
  assert(not ok, "should have errored")
end)

-- 9. pcall catches circular reference
check("pcall on circular ref", function()
  local t = {}
  t.self = t
  local ok, _ = pcall(json.encode, t)
  assert(not ok, "should have errored on circular ref")
end)

-- 10. Unicode escape in string
check("unicode escape", function()
  local t = json.decode('{"emoji":"\\u2764"}')
  assert(type(t.emoji) == "string", "emoji should be string")
  assert(#t.emoji > 0, "emoji should not be empty")
end)

-- 11. VFS write: persist the test result
check("VFS write result", function()
  local result = json.encode({ test = "json", passed = passed + 1, failed = failed })
  local ok = vfs.write("/tmp/json_result.json", result)
  assert(ok, "vfs.write failed")
end)

io.write(string.format("=== %d passed, %d failed ===\n", passed, failed))
if failed == 0 then
  io.write("ALL TESTS PASSED\n")
end
