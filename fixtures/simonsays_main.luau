-- Simon Says MainScript (reconstructed from VisualSource)
--
-- Expected hierarchy under script.Parent:
--   Teleport_Places["Simon says game"]  (Part)
--   Messages: IntermissionValue, ActionValue
--   Configurations: Intermission, MinimalPlayers, Toggle
--   Remotes: IntermissionUI, UIMove, DiedEvent
--   Bindables: GatherWord (BindableFunction), LetPeopleSayWord, GiveWord, GiveAdmin, ToggleEvent
--   Possible_Commands: children named SAW / SSSAW / SSSAWLOD / SF / SSSF / or anything else
--       each with: FakeSimon, Time, LastDies, Message, Object
-- Also: Lighting.Items.LinkedSword, Lighting.CollisionPlaces

local Players = game:GetService("Players")
local lighting = game:FindFirstChild("Lighting")

------------------------------------------------------------------ state
local PlayersSaid = {}
local itemTable = {}
local deleteTable = {}
local PlayerTableBackup = {}
local PlayersInHitbox = {}
local PlayerTable = {}

local gameRunning = false
local timeRunning = false
local intermissionValue = 0
local playerCount = 0       -- original variable "Players" (number)
local PlayersAlive = 0

-- NOTE: the original has TWO different variables, "fakeSimon" and "FakeSimon" (case matters).
-- fakeSimon is used by normal / sword-fight commands, FakeSimon by the SAW / SSSAW / SSSAWLOD word commands.
local fakeSimon = false
local FakeSimon = nil

local telplaces = script.Parent:FindFirstChild("Teleport_Places")
local TelPart = telplaces:FindFirstChild("Simon says game")
local simon_position = TelPart.Position

local collisionPlaces = lighting:FindFirstChild("CollisionPlaces") -- found in the original but never used
local messages = script.Parent:FindFirstChild("Messages")
local config = script.Parent:FindFirstChild("Configurations")
local intermission = config.Intermission.Value
local minPlayers = config.MinimalPlayers.Value                    -- read in the original but never used

local toggle = config.Toggle.Value
config.Toggle:GetPropertyChangedSignal("Value"):Connect(function()
	toggle = config.Toggle.Value
end)

local TEXT_POSITION = UDim2.new(0, 0, 0.4, 0)
local TYPO_PREFIXES = { "simon sais ", "saimon sais ", "simone siys " }

------------------------------------------------------------------ helpers
local function clearTable(t)
	table.clear(t)
end

-- 10% chance to prefix a fake-Simon message with a "typo'd" Simon says
local function maybeTypo(message)
	if math.random(1, 100) <= 10 then
		return TYPO_PREFIXES[math.random(1, 3)] .. message
	end
	return message
end

-- Kills a player (only if they're still on the "alive" team = BrickColor 21)
local function punish(player)
	table.insert(deleteTable, player)
	print(player)
	print("will die")
	if player.TeamColor == BrickColor.new(21) then
		player.Character.Humanoid:TakeDamage(9999)
	end
end

------------------------------------------------------------------ functions
local function addPlayersToTable()
	for _, player in ipairs(Players:GetChildren()) do
		local team = player.TeamColor
		if team ~= BrickColor.new(37) then          -- 37 = winner team
			player.TeamColor = BrickColor.new(21)   -- 21 = alive team
			table.insert(PlayerTable, player)
		end
	end
	PlayersAlive = #PlayerTable
end

local function Intermission()
	script.Parent.Remotes.IntermissionUI:FireAllClients(true)
	timeRunning = true
	local lastTime = os.time()
	while timeRunning == true do
		local nowTime = os.time()
		local result = nowTime - lastTime
		intermissionValue = math.floor(intermission - result)
		if intermissionValue <= 0 then
			timeRunning = false
			messages.IntermissionValue.Value = intermissionValue
			script.Parent.Remotes.IntermissionUI:FireAllClients(false)
		end
		if intermissionValue >= 1 then
			messages.IntermissionValue.Value = intermissionValue
		end
		task.wait() -- added (the block loop had no explicit wait)
	end
end

local function actionTime(Time)
	script.Parent.Remotes.IntermissionUI:FireAllClients(true)
	timeRunning = true
	local lastTime = os.time()
	while timeRunning == true do
		local nowTime = os.time()
		local result = nowTime - lastTime
		intermissionValue = math.floor(Time - result)
		if intermissionValue <= 0 then
			timeRunning = false
			messages.IntermissionValue.Value = intermissionValue
			script.Parent.Remotes.IntermissionUI:FireAllClients(false)
		end
		if intermissionValue >= 1 then
			messages.ActionValue.Value = intermissionValue
		end
		task.wait() -- added
	end
end

-- DiedEvent from clients: remove that player from PlayerTable
script.Parent.Remotes.DiedEvent.OnServerEvent:Connect(function(plr, name, player)
	local key = table.find(PlayerTable, player)
	local value = key and PlayerTable[key] or nil
	if value ~= nil then
		local namenew = value.Name
		if name == namenew then
			table.remove(PlayerTable, key)
			PlayersAlive = PlayersAlive - 1
			player.TeamColor = BrickColor.new(194) -- dead team
		end
	end
	print(plr)
end)

local function SimonText(Position, Text)
	script.Parent.Remotes.UIMove:FireAllClients(Position, Text)
end

Players.PlayerAdded:Connect(function(plr)
	playerCount = playerCount + 1
end)

Players.PlayerRemoving:Connect(function(plr)
	playerCount = playerCount - 1
	local key = table.find(PlayerTable, plr)
	local value = key and PlayerTable[key] or nil
	if value ~= nil then
		table.remove(PlayerTable, key)
		PlayersAlive = PlayersAlive - 1
	end
	print(plr)
end)

local function Teleport(Position)
	for _, player in ipairs(Players:GetChildren()) do
		local value = player.TeamColor
		if value ~= BrickColor.new(37) and value ~= BrickColor.new(194) then
			local char = player.Character
			local root = char and char:FindFirstChild("HumanoidRootPart")
			if root then
				root.CFrame = CFrame.new(Position)
			end
		end
	end
end

script.Parent.Bindables.ToggleEvent.Event:Connect(function()
	config.Toggle.Value = not config.Toggle.Value
end)

------------------------------------------------------------------ hitbox round
-- Clones the command's Part/Model, tracks who stands in it, and after `Time` seconds
-- (or earlier, when EVERY player is inside) judges the players.
local function makePlayersGoToPlace(Part, Time, FakeSimon, LastDies)
	local collisionHitBox

	-- Shared judging block (the original wires this to BOTH the end of the function and the "everyone is inside" check)
	local function evaluate()
		-- backup PlayerTable
		for _, player in ipairs(PlayerTable) do
			table.insert(PlayerTableBackup, player)
		end

		-- look each player up in PlayersInHitbox
		for _, player in ipairs(PlayerTableBackup) do
			local key = table.find(PlayersInHitbox, player)
			local ok = key and PlayersInHitbox[key] or nil
			if FakeSimon == false and ok == nil then
				punish(player)            -- real Simon: must be INSIDE
			end
			if FakeSimon == true and ok ~= nil then
				punish(player)            -- fake Simon: must stay OUTSIDE
			end
		end

		-- "last one dies"
		if LastDies == true then
			clearTable(deleteTable)
			clearTable(PlayerTableBackup)
			for _, player in ipairs(PlayerTable) do
				table.insert(PlayerTableBackup, player)
			end
			task.wait(0.1)
			for _, player in ipairs(PlayerTableBackup) do
				if table.find(PlayersInHitbox, player) == nil then
					table.insert(deleteTable, player)
				end
			end
			local length = #PlayersInHitbox
			local lastIn = PlayersInHitbox[length]
			if length >= 2 and lastIn then
				lastIn.Character.Humanoid.Health = 0
			end
		end

		-- clear tables, destroy hitbox
		clearTable(PlayerTableBackup)
		clearTable(deleteTable)
		clearTable(PlayersInHitbox)
		if collisionHitBox then
			collisionHitBox:Destroy()
		end
	end

	print("Cloning")
	collisionHitBox = Part:Clone()
	collisionHitBox.Parent = workspace

	local function track(hitbox, onAllInside)
		local touchConn, endConn
		touchConn = hitbox.Touched:Connect(function(hit)
			if hit.Name == "Torso" then
				local plr = Players:GetPlayerFromCharacter(hit.Parent)
				if plr then
					table.insert(PlayersInHitbox, plr)
					print("added")
					print(PlayersInHitbox)
					if onAllInside and #PlayersInHitbox == #PlayerTable then
						touchConn:Disconnect()
						endConn:Disconnect()
						onAllInside()
					end
				end
			end
		end)
		endConn = hitbox.TouchEnded:Connect(function(hit)
			if hit.Name == "Torso" then
				local plr = Players:GetPlayerFromCharacter(hit.Parent)
				local key = table.find(PlayersInHitbox, plr)
				if key then
					table.remove(PlayersInHitbox, key)
					print("removed")
					print(PlayersInHitbox)
				end
			end
		end)
		return touchConn, endConn
	end

	if Part:IsA("Model") == false then
		-- single part: only this one finishes early when everyone is inside
		local touchConn, endConn = track(collisionHitBox, evaluate)
		task.wait(Time)
		touchConn:Disconnect()
		endConn:Disconnect()
	else
		-- model made of "BlackLineParts" (two sub-parts)
		if Part.Name == "BlackLineParts" then
			local t1, e1 = track(collisionHitBox.BlackLinePart)
			local t2, e2 = track(collisionHitBox.BlackLinePart2)
			task.wait(Time)
			t1:Disconnect(); e1:Disconnect()
			t2:Disconnect(); e2:Disconnect()
		end
	end

	evaluate()
end

------------------------------------------------------------------ word rounds (SAW / SSSAW / SSSAWLOD)
local function runWordRound(isFake, lastDies)
	script.Parent.Bindables.LetPeopleSayWord:Fire(15)

	local word = ""
	return function(wordToSay)
		word = wordToSay
		local conn
		conn = script.Parent.Bindables.GiveWord.Event:Connect(function(wordNew, player)
			if string.lower(wordNew) == string.lower(word) then
				table.insert(PlayersSaid, player)
			end
		end)
		task.wait(15)
		conn:Disconnect()

		-- backup PlayerTable
		for _, player in ipairs(PlayerTable) do
			table.insert(PlayerTableBackup, player)
		end

		-- judge
		for _, player in ipairs(PlayerTableBackup) do
			local key = table.find(PlayersSaid, player)
			local ok = key and PlayersSaid[key] or nil
			if isFake == false and ok == nil then
				punish(player)       -- real: must have said it
			end
			if isFake == true and ok ~= nil then
				punish(player)       -- fake: must NOT say it
			end
		end

		-- last speaker dies
		if lastDies == true then
			clearTable(deleteTable)
			clearTable(PlayerTableBackup)
			for _, player in ipairs(PlayerTable) do
				table.insert(PlayerTableBackup, player)
			end
			task.wait(0.1)
			for _, player in ipairs(PlayerTableBackup) do
				if table.find(PlayersSaid, player) == nil then
					table.insert(deleteTable, player)
				end
			end
			local length = #PlayersSaid
			local lastSpeaker = PlayersSaid[length]
			if length >= 2 and lastSpeaker then
				lastSpeaker.Character.Humanoid.Health = 0
			end
		end

		-- clear tables
		clearTable(PlayerTableBackup)
		clearTable(deleteTable)
		clearTable(PlayersSaid)
	end
end

------------------------------------------------------------------ sword fight
local SWORD_FIGHT_TIME = 30

local function runSwordFight(killOnDamage)
	task.spawn(actionTime, SWORD_FIGHT_TIME)
	timeRunning = true
	local lastTime = os.time()

	for _, player in ipairs(PlayerTable) do
		local sword = lighting.Items.LinkedSword:Clone()
		sword.Parent = player.Backpack
		if killOnDamage then
			sword.SwordScript.KillOnDamage.Value = true
		end
		table.insert(itemTable, sword)
	end

	while timeRunning == true do
		local nowTime = os.time()
		local result = nowTime - lastTime
		intermissionValue = math.floor(SWORD_FIGHT_TIME - result)
		if intermissionValue <= 0 then
			timeRunning = false
		end
		if PlayersAlive <= 1 then
			timeRunning = false
		end
		task.wait() -- added
	end

	for i = #itemTable, 1, -1 do
		local value = itemTable[i]
		if value ~= nil then
			value:Destroy()
		end
		table.remove(itemTable, i)
	end
end

------------------------------------------------------------------ the game
local function Game()
	local message = ""
	fakeSimon = false
	gameRunning = true

	-- (original: While gameRunning -> AND(gameRunning, toggle) -> inner While; merged into one loop)
	while gameRunning and toggle do
		-- drop nil holes
		for place = #PlayerTable, 1, -1 do
			if PlayerTable[place] == nil then
				table.remove(PlayerTable, place) -- original referenced "val" here, which was a bug
			end
		end

		-- game switched off from config
		if config.Toggle.Value == false then
			gameRunning = false
			for i = #PlayerTable, 1, -1 do
				table.remove(PlayerTable, i)
			end
			break
		end

		task.wait(25)

		if PlayersAlive >= 2 then
			local commands = script.Parent.Possible_Commands:GetChildren()
			local cmd = commands[math.random(1, #commands)]
			local name = cmd.Name
			fakeSimon = cmd:FindFirstChild("FakeSimon").Value
			local time = cmd:FindFirstChild("Time").Value
			local LastDies = cmd:FindFirstChild("LastDies").Value
			message = cmd:FindFirstChild("Message").Value
			local collision = cmd:FindFirstChild("Object").Value

			if name == "SSSF" then
				-- Simon says sword fight
				task.spawn(SimonText, TEXT_POSITION, "Simon says Sword fight!")
				runSwordFight(false)

			elseif name == "SF" then
				-- fake sword fight (sword kills on damage)
				fakeSimon = true
				message = "Sword fight"
				task.spawn(SimonText, TEXT_POSITION, maybeTypo(message))
				runSwordFight(true)

			elseif name == "SAW" then
				-- fake "Say <word>"
				FakeSimon = true
				task.spawn(actionTime, 30)
				local word = script.Parent.Bindables.GatherWord:Invoke()
				message = "Say " .. word
				if FakeSimon == true then
					task.spawn(SimonText, TEXT_POSITION, maybeTypo(message))
				end
				LastDies = false
				runWordRound(FakeSimon, LastDies)(word)

			elseif name == "SSSAW" then
				-- Simon says say <word>
				FakeSimon = false
				task.spawn(actionTime, 15)
				local word = script.Parent.Bindables.GatherWord:Invoke()
				message = "Simon says say " .. word
				if FakeSimon == false then
					task.spawn(SimonText, TEXT_POSITION, message)
				end
				LastDies = false
				runWordRound(FakeSimon, LastDies)(word)

			elseif name == "SSSAWLOD" then
				-- Simon says say <word>, last one dies.
				FakeSimon = false
				task.spawn(actionTime, 15)
				local word = script.Parent.Bindables.GatherWord:Invoke()
				message = "Simon says say " .. word
				message = message .. ", last one dies."
				if FakeSimon == false then
					task.spawn(SimonText, TEXT_POSITION, message)
				end
				LastDies = true
				runWordRound(FakeSimon, LastDies)(word)

			else
				-- normal hitbox command
				if fakeSimon == true then
					task.spawn(SimonText, TEXT_POSITION, maybeTypo(message))
					task.spawn(makePlayersGoToPlace, collision, 60, true, LastDies)
					actionTime(time)
				end
				if fakeSimon == false then
					task.spawn(makePlayersGoToPlace, collision, 60, false, LastDies)
					task.spawn(SimonText, TEXT_POSITION, message)
					actionTime(time)
				end
			end
		end

		-- someone won?
		if PlayersAlive <= 1 then
			local length = #PlayerTable
			local value = PlayerTable[length]
			if value ~= nil then
				local human = value.Character and value.Character:FindFirstChild("Humanoid")
				if human ~= nil then
					value.TeamColor = BrickColor.new(37)
					local text = value.Name .. " Won!"
					task.spawn(SimonText, TEXT_POSITION, text)
					task.wait(5)
					script.Parent.Bindables.GiveAdmin:Fire(value)
				end
			end
			gameRunning = false
			for i = #PlayerTable, 1, -1 do
				table.remove(PlayerTable, i)
			end
			break
		end
	end
end

------------------------------------------------------------------ main loop
while toggle == true do
	if playerCount >= 4 then -- (config MinimalPlayers is read above but the original hardcodes 4)
		local valueok = config.Toggle.Value
		if valueok == true then
			Intermission()
			addPlayersToTable()
			Teleport(simon_position)
			Game()
		end
	end
	task.wait(1) -- added so the loop doesn't spin when there aren't enough players
end
