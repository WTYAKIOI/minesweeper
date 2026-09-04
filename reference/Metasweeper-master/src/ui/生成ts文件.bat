set HISTORY_PLUGIN=..\plugins\History\plugin.py ..\plugins\History\main_widget.py ..\plugins\History\history_table.py ..\plugins\History\table_views.py ..\plugins\History\models.py ..\plugins\History\filter_dialog.py ..\plugins\History\sort_dialog.py ..\plugins\History\columns_dialog.py
set LLM_PLUGIN=..\plugins\llm_minesweeper_controller\config.py ..\plugins\llm_minesweeper_controller\widgets.py ..\plugins\llm_minesweeper_controller\plugin.py ..\plugins\llm_minesweeper_controller\api_client.py ..\plugins\llm_minesweeper_controller\function_registry.py
set XIANNI_PLUGIN=..\plugins\XianNiUpgrade\plugin.py ..\plugins\XianNiUpgrade\widgets.py ..\plugins\XianNiUpgrade\models.py
set PLUGIN_MANAGER=..\plugin_manager\main_window.py
set DIALOGS=..\dialogs\videoControl.py ..\dialogs\gameScores.py
set REPLAY_ANALYSIS=..\replay_analysis\guess.py ..\replay_analysis\one_point_five_click.py ..\replay_analysis\combo_click.py ..\replay_analysis\flag.py

pylupdate5 ui_gameSettings.py ui_main_board.py ui_about.py ui_defined_parameter.py ui_gameSettingShortcuts.py ui_score_board.py ui_record_pop.py ui_advanced.py ui_video_control.py ui_import.py %DIALOGS% %REPLAY_ANALYSIS% ../main.py ../mainWindowGUIImportExport.py ../mineSweeperGUI.py ../mineSweeperGUIEvent.py ../mineSweeperVideoPlayer.py ../shared_types/enums.py ../shared_types/widgets/confirm_dialog.py ../utils/helpers.py %HISTORY_PLUGIN% %LLM_PLUGIN% %XIANNI_PLUGIN% %PLUGIN_MANAGER% -ts en_US.ts -noobsolete

pylupdate5 ui_gameSettings.py ui_main_board.py ui_about.py ui_defined_parameter.py ui_gameSettingShortcuts.py ui_score_board.py ui_record_pop.py ui_advanced.py ui_video_control.py ui_import.py %DIALOGS% %REPLAY_ANALYSIS% ../main.py ../mainWindowGUIImportExport.py ../mineSweeperGUI.py ../mineSweeperGUIEvent.py ../mineSweeperVideoPlayer.py ../shared_types/enums.py ../shared_types/widgets/confirm_dialog.py ../utils/helpers.py %HISTORY_PLUGIN% %LLM_PLUGIN% %XIANNI_PLUGIN% %PLUGIN_MANAGER% -ts pl_PL.ts -noobsolete

pylupdate5 ui_gameSettings.py ui_main_board.py ui_about.py ui_defined_parameter.py ui_gameSettingShortcuts.py ui_score_board.py ui_record_pop.py ui_advanced.py ui_video_control.py ui_import.py %DIALOGS% %REPLAY_ANALYSIS% ../main.py ../mainWindowGUIImportExport.py ../mineSweeperGUI.py ../mineSweeperGUIEvent.py ../mineSweeperVideoPlayer.py ../shared_types/enums.py ../shared_types/widgets/confirm_dialog.py ../utils/helpers.py %HISTORY_PLUGIN% %LLM_PLUGIN% %XIANNI_PLUGIN% %PLUGIN_MANAGER% -ts de_DE.ts -noobsolete

pylupdate5 ui_gameSettings.py ui_main_board.py ui_about.py ui_defined_parameter.py ui_gameSettingShortcuts.py ui_score_board.py ui_record_pop.py ui_advanced.py ui_video_control.py ui_import.py %DIALOGS% %REPLAY_ANALYSIS% ../main.py ../mainWindowGUIImportExport.py ../mineSweeperGUI.py ../mineSweeperGUIEvent.py ../mineSweeperVideoPlayer.py ../shared_types/enums.py ../shared_types/widgets/confirm_dialog.py ../utils/helpers.py %HISTORY_PLUGIN% %LLM_PLUGIN% %XIANNI_PLUGIN% %PLUGIN_MANAGER% -ts ja_JP.ts -noobsolete



@REM lrelease en_US.ts
@REM lrelease pl_PL.ts
@REM lrelease de_DE.ts
@REM lrelease ja_JP.ts
