    var strScreenshotURL = "%dl_screenshot_url%";
    var strRemoteClickURL = "%remote_screenshot_click_url%";
    var strPNGFile = "%dl_screenshot_title%" + ".png";
    var strGameText = "%game_text%";
    var strGameName = "%dl_screenshot_gamename%";
    var bHasIPPort = %dl_screenshot_hasipport%;
    var strServerIPText = "%text_serverip%";
    var strIPPort = "%dl_screenshot_ipport%";
    var strTimeTakenText = "%time_taken_text%";
    var strTimeTaken = "%dl_screenshot_timestamp%";
    var strDescriptionText = "%description_text%";
    var strDescription = "%dl_screenshot_description%";
    var strResolutionText = "%resolution_text%";
    var strDimensions = "%dl_screenshot_dimensions%";
    var strFileSizeText = "%text_totalsize%";
    var strFileSize = "%dl_screenshot_filesize%";
    var bIsRemote = %is_remote_screenshot%;
    var strLocalText = "%local_text%";
    var strRemoteText = "%remote_text%";
    var strRemoteQuota = "%remote_quota_usage%";
    var strNumLocalScreenshots = "%num_local_screenshots%";
    var strLocalIcon = "%media_template_folder%infoview/images/icon_local_screenshot.gif";
    var strRemoteIcon = "%media_template_folder%infoview/images/icon_remote_screenshot.gif";
    var strSaveText = "%save_text%";
    var bDisconnected = %is_disconnected%;
    var strStatusText = "%text_status%";
    var strStatus = "%media_upload_status%";
    var bDisplayUploadStatus = %display_media_upload_status%;

    function GetDescription()
    {
		var textarea_element = document.getElementById("desc_id");
		if (textarea_element)
		{
			if (textarea_element.value.length > 1024)
			{
				return textarea_element.value.substr(0, 1024);
			}
			else
			{
				return textarea_element.value;
			}
		}
		return "";
    }

    function RenderHeader()
    {
		var bDisconnected = %is_disconnected%;
		if (bDisconnected)
		{
			show_element("remote_section_id", false);
		}
    }
