-- coroutine_test.lua — validates setjmp/longjmp context switching in ViCell
-- Run: lua /tmp/coroutine_test.lua

io.write("=== coroutine_test on ViCell Lua 5.4 ===\n")

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

-- 1. Basic yield/resume
check("yield and resume", function()
  local co = coroutine.create(function()
    coroutine.yield(10)
    coroutine.yield(20)
    return 30
  end)
  local ok1, v1 = coroutine.resume(co)
  local ok2, v2 = coroutine.resume(co)
  local ok3, v3 = coroutine.resume(co)
  assert(ok1 and v1 == 10, "first yield")
  assert(ok2 and v2 == 20, "second yield")
  assert(ok3 and v3 == 30, "return value")
  assert(coroutine.status(co) == "dead", "should be dead")
end)

-- 2. Producer-consumer via coroutine
check("producer consumer", function()
  local function producer(n)
    for i = 1, n do
      coroutine.yield(i * i)
    end
  end

  local co = coroutine.create(producer)
  local results = {}
  local ok, v = coroutine.resume(co, 5)
  while ok and v ~= nil do
    table.insert(results, v)
    ok, v = coroutine.resume(co)
  end
  assert(#results == 5,     "#results=" .. #results)
  assert(results[1] == 1,   "1^2=1")
  assert(results[3] == 9,   "3^2=9")
  assert(results[5] == 25,  "5^2=25")
end)

-- 3. Coroutine wrap
check("coroutine.wrap", function()
  local gen = coroutine.wrap(function()
    for i = 1, 3 do coroutine.yield(i) end
  end)
  assert(gen() == 1, "wrap 1")
  assert(gen() == 2, "wrap 2")
  assert(gen() == 3, "wrap 3")
end)

-- 4. Value passing into resume
check("pass values into resume", function()
  local co = coroutine.create(function(a, b)
    local c = coroutine.yield(a + b)
    return c * 2
  end)
  local ok1, sum  = coroutine.resume(co, 3, 4)  -- a=3 b=4
  local ok2, prod = coroutine.resume(co, 10)      -- c=10
  assert(ok1 and sum  == 7,  "sum=" .. tostring(sum))
  assert(ok2 and prod == 20, "prod=" .. tostring(prod))
end)

-- 5. Error inside coroutine is caught, coroutine dies
check("error in coroutine", function()
  local co = coroutine.create(function()
    error("oops from coroutine")
  end)
  local ok, msg = coroutine.resume(co)
  assert(not ok, "should fail")
  assert(type(msg) == "string" and msg:find("oops"), "msg=" .. tostring(msg))
  assert(coroutine.status(co) == "dead", "should be dead after error")
end)

-- 6. Resuming dead coroutine returns false
check("resume dead coroutine", function()
  local co = coroutine.create(function() return 1 end)
  coroutine.resume(co)  -- runs to completion
  local ok, msg = coroutine.resume(co)
  assert(not ok, "should fail on dead coroutine")
  assert(msg ~= nil, "should have error message")
end)

-- 7. coroutine.running inside a coroutine
check("coroutine.running", function()
  local inner_co
  local co = coroutine.create(function()
    inner_co = coroutine.running()
  end)
  coroutine.resume(co)
  assert(inner_co == co, "running() should return the coroutine itself")
end)

-- 8. Many yields (stress-test stack switching)
check("100 yields", function()
  local sum = 0
  local co = coroutine.create(function()
    for i = 1, 100 do coroutine.yield(i) end
  end)
  local ok, v = coroutine.resume(co)
  while ok and v ~= nil do
    sum = sum + v
    ok, v = coroutine.resume(co)
  end
  assert(sum == 5050, "1+2+...+100 = 5050, got " .. sum)
end)

io.write(string.format("=== %d passed, %d failed ===\n", passed, failed))
if failed == 0 then
  io.write("ALL TESTS PASSED\n")
end
