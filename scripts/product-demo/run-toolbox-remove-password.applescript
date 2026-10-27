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
		set candidates to my accessibleDescendants(window "Toolbox" of toolboxProcess)
		repeat with candidate in candidates
			try
				if (role of candidate as text) is "AXButton" and ((name of candidate as text) contains "Remove Password from selected files" or (description of candidate as text) contains "Remove Password from selected files") then
					perform action "AXPress" of candidate
					delay 1
					return
				end if
			end try
		end repeat
	end tell
	error "Could not click Remove Password from selected files"
end run
