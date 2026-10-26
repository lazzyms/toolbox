; Removes the Explorer context-menu entries the app registers on launch (src/quick_action.rs).
!macro NSIS_HOOK_PREUNINSTALL
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.pdf\shell\Toolbox"
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.png\shell\Toolbox"
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.jpg\shell\Toolbox"
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.jpeg\shell\Toolbox"
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.webp\shell\Toolbox"
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.heic\shell\Toolbox"
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.heif\shell\Toolbox"
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.gif\shell\Toolbox"
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.tif\shell\Toolbox"
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.tiff\shell\Toolbox"
!macroend
