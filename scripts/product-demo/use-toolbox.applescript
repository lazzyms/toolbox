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

on findAndPress(processRef, targetRole, labelText)
	tell application "System Events"
		set candidates to my accessibleDescendants(my toolboxWindow(processRef))
		repeat with candidate in candidates
			try
				set candidateRole to role of candidate as text
				set candidateName to ""
				set candidateDescription to ""
				set candidateEnabled to true
				try
					set candidateName to name of candidate as text
				end try
				try
					set candidateDescription to description of candidate as text
				end try
				try
					set candidateEnabled to enabled of candidate
				end try
				if candidateRole is targetRole and candidateEnabled and (candidateName contains labelText or candidateDescription contains labelText) then
					perform action "AXPress" of candidate
					return true
				end if
			end try
		end repeat
	end tell
	return false
end findAndPress

-- Controls stay disabled while the editor loads or renders its export preview.
on pressWhenReady(processRef, targetRole, labelText)
	repeat 30 times
		if my findAndPress(processRef, targetRole, labelText) then return true
		delay 1
	end repeat
	return false
end pressWhenReady

on enterTextInField(processRef, labelText, fieldText)
	tell application "System Events"
		set candidates to my accessibleDescendants(my toolboxWindow(processRef))
		repeat with candidate in candidates
			try
				set candidateRole to role of candidate as text
				set candidateName to ""
				set candidateDescription to ""
				try
					set candidateName to name of candidate as text
				end try
				try
					set candidateDescription to description of candidate as text
				end try
				try
					set candidateValue to value of candidate as text
				end try
				if candidateRole is "AXTextField" and (candidateName contains labelText or candidateDescription contains labelText or candidateValue contains labelText) then
					perform action "AXPress" of candidate
					keystroke "a" using {command down}
					keystroke fieldText
					return true
				end if
			end try
		end repeat
	end tell
	return false
end enterTextInField

on run argv
	if (count of argv) is not 2 then error "Pass the Toolbox process ID and absolute path to the demo PDF"
	set processID to item 1 of argv as integer
	set fixturePath to item 2 of argv

	tell application "System Events"
		set toolboxProcess to first application process whose unix id is processID
		set frontmost of toolboxProcess to true
		delay 3

		if my enterTextInField(toolboxProcess, "Search tools", "Edit PDF") is false then
			error "Could not search the Toolbox library"
		end if
		delay 0.4

		if my findAndPress(toolboxProcess, "AXButton", "Edit PDF") is false then
			error "Could not open Edit PDF from the Toolbox library"
		end if
		delay 1

		if my findAndPress(toolboxProcess, "AXButton", "Choose files to process") is false then
			error "Could not open the PDF picker"
		end if
		delay 1

		keystroke "g" using {command down, shift down}
		delay 0.3
		keystroke fixturePath
		key code 36
		delay 0.5
		key code 36
		delay 1

		if my pressWhenReady(toolboxProcess, "AXCheckBox", "Watermark") is false then
			error "Could not choose the PDF editor watermark tool"
		end if
		delay 0.4

		if my findAndPress(toolboxProcess, "AXButton", "Add fixed watermark") is false then
			error "Could not add the PDF annotation"
		end if
		delay 0.6

		if my enterTextInField(toolboxProcess, "Watermark text", "Files stay on this device.") is false then
			error "Could not enter the PDF annotation text"
		end if
		delay 0.2

		if my pressWhenReady(toolboxProcess, "AXButton", "Export PDF") is false then
			error "Could not export the edited PDF"
		end if
	end tell
end run
