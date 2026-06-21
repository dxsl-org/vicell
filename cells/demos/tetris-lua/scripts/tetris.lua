-- ViCell Tetris — Lua edition
-- Platform globals:
--   surface.fill_rect(x,y,w,h,ci)   surface.print(x,y,s,ci,scale)
--   surface.flush()  surface.width()  surface.height()
--   input.poll_key()  time.ticks()
-- Key codes: 0=none 1=left 2=right 3=rotate 4=down 5=hard_drop 6=quit

local COLS, ROWS = 10, 20
local CELL = 28
local SW, SH = surface.width(), surface.height()
local OX = math.floor((SW - COLS*CELL)/2) - 90
local OY = math.floor((SH - ROWS*CELL)/2)
local SX = OX + COLS*CELL + 18

local C_BG, C_BORD, C_TEXT = 0, 8, 15

-- 7 tetrominoes: {ci=color_index, r={4 rotations, each = 4 {col,row} in 4×4 grid}}
local PIECES = {
  {ci=11,r={{{0,1},{1,1},{2,1},{3,1}},{{2,0},{2,1},{2,2},{2,3}},{{0,2},{1,2},{2,2},{3,2}},{{1,0},{1,1},{1,2},{1,3}}}},
  {ci=9, r={{{0,0},{0,1},{1,1},{2,1}},{{1,0},{2,0},{1,1},{1,2}},{{0,1},{1,1},{2,1},{2,2}},{{1,0},{1,1},{0,2},{1,2}}}},
  {ci=14,r={{{2,0},{0,1},{1,1},{2,1}},{{1,0},{1,1},{1,2},{2,2}},{{0,1},{1,1},{2,1},{0,2}},{{0,0},{1,0},{1,1},{1,2}}}},
  {ci=6, r={{{1,0},{2,0},{1,1},{2,1}},{{1,0},{2,0},{1,1},{2,1}},{{1,0},{2,0},{1,1},{2,1}},{{1,0},{2,0},{1,1},{2,1}}}},
  {ci=10,r={{{1,0},{2,0},{0,1},{1,1}},{{1,0},{1,1},{2,1},{2,2}},{{1,1},{2,1},{0,2},{1,2}},{{0,0},{0,1},{1,1},{1,2}}}},
  {ci=13,r={{{1,0},{0,1},{1,1},{2,1}},{{1,0},{1,1},{2,1},{1,2}},{{0,1},{1,1},{2,1},{1,2}},{{1,0},{0,1},{1,1},{1,2}}}},
  {ci=12,r={{{0,0},{1,0},{1,1},{2,1}},{{2,0},{1,1},{2,1},{1,2}},{{0,1},{1,1},{1,2},{2,2}},{{1,0},{0,1},{1,1},{0,2}}}},
}

local board = {}
for r=1,ROWS do board[r]={} for c=1,COLS do board[r][c]=0 end end

-- 7-bag random
local bag, bagi = {1,2,3,4,5,6,7}, 8
local function next_kind()
  if bagi>7 then
    bag={1,2,3,4,5,6,7}; bagi=1
    for i=7,2,-1 do local j=math.random(i); bag[i],bag[j]=bag[j],bag[i] end
  end
  local k=bag[bagi]; bagi=bagi+1; return k
end
math.randomseed(time.ticks())

local pk,pr,pc,prow,nk
local score,nlines,lvl=0,0,1
local done=false
local DROP_MS={800,720,630,550,470,380,300,220,130,100,80,80,80,70,70,70,50,50,50,30}
local function drop_ms() return DROP_MS[math.min(lvl,#DROP_MS)] end

local function cells(kind,rot,col,row)
  local out={}
  for _,d in ipairs(PIECES[kind].r[rot]) do out[#out+1]={col+d[1],row+d[2]} end
  return out
end

local function fits(kind,rot,col,row)
  for _,c in ipairs(cells(kind,rot,col,row)) do
    local x,y=c[1],c[2]
    if x<1 or x>COLS or y>ROWS then return false end
    if y>=1 and board[y][x]~=0 then return false end
  end
  return true
end

local function spawn()
  pk=nk or next_kind(); nk=next_kind()
  pr=1; pc=3; prow=0
  if not fits(pk,pr,pc,prow) then done=true end
end

local function lock_clear()
  local ci=PIECES[pk].ci
  for _,c in ipairs(cells(pk,pr,pc,prow)) do
    if c[2]>=1 then board[c[2]][c[1]]=ci end
  end
  local cleared=0; local r=ROWS
  while r>=1 do
    local full=true
    for c=1,COLS do if board[r][c]==0 then full=false; break end end
    if full then
      table.remove(board,r); table.insert(board,1,{})
      for c=1,COLS do board[1][c]=0 end; cleared=cleared+1
    else r=r-1 end
  end
  nlines=nlines+cleared
  score=score+({0,100,300,500,800})[cleared+1]*lvl
  lvl=math.floor(nlines/10)+1
end

local function draw_cell(bx,by,ci)
  surface.fill_rect(bx,by,CELL,CELL,C_BORD)
  surface.fill_rect(bx+1,by+1,CELL-2,CELL-2,ci)
end

local function draw()
  surface.fill_rect(0,0,SW,SH,C_BG)
  surface.fill_rect(OX-2,OY-2,COLS*CELL+4,ROWS*CELL+4,C_BORD)
  surface.fill_rect(OX,OY,COLS*CELL,ROWS*CELL,C_BG)
  for r=1,ROWS do
    for c=1,COLS do
      if board[r][c]~=0 then draw_cell(OX+(c-1)*CELL,OY+(r-1)*CELL,board[r][c]) end
    end
  end
  if not done then
    -- Ghost piece
    local gr=prow
    while fits(pk,pr,pc,gr+1) do gr=gr+1 end
    if gr~=prow then
      for _,p in ipairs(cells(pk,pr,pc,gr)) do
        if p[2]>=1 then surface.fill_rect(OX+(p[1]-1)*CELL+3,OY+(p[2]-1)*CELL+3,CELL-6,CELL-6,C_BORD) end
      end
    end
    -- Active piece
    for _,p in ipairs(cells(pk,pr,pc,prow)) do
      if p[2]>=1 then draw_cell(OX+(p[1]-1)*CELL,OY+(p[2]-1)*CELL,PIECES[pk].ci) end
    end
  end
  -- Score panel
  surface.print(SX,OY+0,   "SCORE",C_TEXT,2)
  surface.print(SX,OY+24,  tostring(score),14,2)
  surface.print(SX,OY+64,  "LINES",C_TEXT,2)
  surface.print(SX,OY+88,  tostring(nlines),14,2)
  surface.print(SX,OY+128, "LEVEL",C_TEXT,2)
  surface.print(SX,OY+152, tostring(lvl),14,2)
  -- Next piece preview
  surface.print(SX,OY+192, "NEXT",C_TEXT,2)
  local s=13
  for _,d in ipairs(PIECES[nk].r[1]) do
    surface.fill_rect(SX+d[1]*s,OY+220+d[2]*s,s-1,s-1,PIECES[nk].ci)
  end
  if done then
    surface.fill_rect(OX,OY+ROWS*CELL/2-20,COLS*CELL,40,0)
    surface.print(OX+4,OY+ROWS*CELL/2-8,"GAME OVER",12,2)
  end
  surface.flush()
end

spawn()
draw()
local last_drop=time.ticks()

while not done do
  local now=time.ticks()
  local key=input.poll_key()
  if     key==1 and fits(pk,pr,pc-1,prow) then pc=pc-1; draw()
  elseif key==2 and fits(pk,pr,pc+1,prow) then pc=pc+1; draw()
  elseif key==3 then
    local nr=pr%4+1
    if fits(pk,nr,pc,prow) then pr=nr; draw() end
  elseif key==4 and fits(pk,pr,pc,prow+1) then prow=prow+1; last_drop=now; draw()
  elseif key==5 then
    while fits(pk,pr,pc,prow+1) do prow=prow+1 end
    lock_clear(); spawn(); last_drop=now; draw()
  elseif key==6 then done=true
  end
  if now-last_drop >= drop_ms() then
    if fits(pk,pr,pc,prow+1) then prow=prow+1
    else lock_clear(); spawn() end
    last_drop=now; draw()
  end
end

draw()
while input.poll_key()==0 do end
