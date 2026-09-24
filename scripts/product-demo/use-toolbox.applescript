on findAndPress(processRef, targetRole, labelText)
	tell application "System Events"
		set candidates to entire contents of window 1 of processRef
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
				if candidateRole is targetRole and (candidateName contains labelText or candidateDescription contains labelText) then
					perform action "AXPress" of candidate
					return true
				end if
			end try
		end repeat
	end tell
	return false
end findAndPress

on enterTextInField(processRef, labelText, fieldText)
	tell application "System Events"
		set candidates to entire contents of window 1 of processRef
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
				if candidateRole is "AXTextField" and (candidateName contains labelText or candidateDescription contains labelText) then
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
	if (count of argv) is not 1 then error "Pass the absolute path to the demo PDF"
	set fixturePath to item 1 of argv

	tell application "System Events"
		set toolboxProcess to first application process whose bundle identifier is "com.toolbox.desktop"
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

		if my enterTextInField(toolboxProcess, "Edit text", "Files stay on this device.") is false then
			error "Could not enter the PDF annotation text"
		end if
		delay 0.2

		if my findAndPress(toolboxProcess, "AXButton", "Edit PDF") is false then
			error "Could not run the PDF edit"
		end if
	end tell
end run
