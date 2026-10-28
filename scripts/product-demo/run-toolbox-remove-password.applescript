-- Toolbox retitles its window "Toolbox — <view or file>" once a tool opens.
on toolboxWindow(processRef)
	tell application "System Events" to return first window of processRef whose name starts with "Toolbox"
end toolboxWindow

on accessibleDescendants(parentElement)
	tell application "System Events" to set childElements to UI elements of parentElement
	set allElements to {}
	repeat with childElement in childElements
		set childRef to contents of childElement
		set end of allElements to childRef
		try
			set nestedElements to my accessibleDescendants(childRef)
			repeat with nestedElement in nestedElements
				set end of allElements to contents of nestedElement
			end repeat
		end try
	end repeat
	return allElements
end accessibleDescendants

on run argv
	if (count of argv) is not 1 then error "Pass the Toolbox process ID"
	set processID to item 1 of argv as integer
	tell application "System Events"
		set toolboxProcess to first application process whose unix id is processID
		set frontmost of toolboxProcess to true
		delay 0.3
		-- "Unlock files" stays disabled until the picked files and password are applied.
		repeat 15 times
			set candidates to my accessibleDescendants(my toolboxWindow(toolboxProcess))
			repeat with candidate in candidates
				try
					if (role of candidate as text) is "AXButton" and (enabled of candidate) and ((name of candidate as text) contains "Unlock files" or (description of candidate as text) contains "Unlock files") then
						perform action "AXPress" of candidate
						delay 1
						return
					end if
				end try
			end repeat
			delay 1
		end repeat
	end tell
	error "Could not click Unlock files"
end run
