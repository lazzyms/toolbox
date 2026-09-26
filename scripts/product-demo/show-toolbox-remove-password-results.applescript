on run argv
	if (count of argv) is not 1 then error "Pass the Toolbox process ID"
	set processID to item 1 of argv as integer
	tell application "System Events"
		set toolboxProcess to first application process whose unix id is processID
		set frontmost of toolboxProcess to true
		delay 0.3
		key code 121
		delay 0.5
	end tell
end run
