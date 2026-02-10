	/////////////////////////////////////////////////////////////////////
	// video.js
	// - Common video related javascript functions
	/////////////////////////////////////////////////////////////////////
		
    function RenderHeader()
    {
		var bDisconnected = %is_disconnected%;
		if (bDisconnected)
		{
			show_element("remote_section_id", false);
		}
    }
    
    function GetVideoSearchText()
    {
		var element = document.getElementById("video_search_text");
		if (element)
		{
			if (element.value.length > 2048)
			{
				return element.value.substr(0, 2048);
			}
			else
			{
				return element.value;
			}
		}
		return "";	
    }

////////// FROM VIDEO.TMPL ////////////////
    var strVideoURL = "%video_url%";
    var strGameText = "%game_text%";
    var strGameName = "%video_gamename%";
    var strTimeTakenText = "%time_taken_text%";
    var strTimeTaken = "%video_timestamp%";
    var strFileSizeText = "%text_totalsize%";
    var strFileSize = "%video_filesize%";
    var strLengthText = "%text_length%";
    var strLength = "%video_length%";
    var strStatusText = "%text_status%";
    var strStatus = "%media_upload_status%";
    var bDisplayUploadStatus = %display_media_upload_status%;
    var strRemoteVideoUrl = "%remote_video_url%";
    var strRemoteVideoThumbUrl = "%remote_video_thumb_url%";
    var strResolutionText = "%resolution_text%";
    var strResolution = "%video_resolution%";
    var bIsRemote = %is_remote_video%;
    var strTitleText = "%text_title%";
    var strTitle = "%video_title%";
    var strDescriptionText = "%description_text%";
    var strDescription = "%video_description%";
    var strSaveText = "%save_text%";
    var bDisconnected = %is_disconnected%;
    var bDisplayVideoThumb = %display_video_thumb%;
    var bCanUpload = %can_upload%;
    var bHasVideoContests = %has_video_contests%;
	var video_contests = [ %video_contests% ];
    
    function RenderVideoTitle()
    {
	    var element = document.getElementById("video_title_id");
	    if (element)
	    {
	        if (bIsRemote)
	        {
	            element.innerHTML = "%js:video_title%";
	        }
	        else
	        {
				var nIndex = strVideoURL.lastIndexOf("\\");
				if (nIndex != -1)
				{
					element.innerHTML = strVideoURL.substring(nIndex + 1);
				}
				else
				{
    	            element.innerHTML = "Local Video";
				}
	        }
	    }
    }

    function RenderVideoDescription()
    {
        var element = document.getElementById("desc_id");
        if (element)
        {
            element.value = strDescription;
        }
    }
    function RenderVideoDetails()
    {
        if (bDisplayUploadStatus == false)
        {
            show_element("upload_status_id", false);
        }

        if (!bCanUpload)
        {
            show_element("video_upload", false);
        }
        
        if (bIsRemote)
        {
            if (bDisconnected)
            {
                document.getElementById("title_id").disabled = true;
                document.getElementById("desc_id").disabled = true;
                document.getElementById("save_id").disabled = true;
            }
        }
        else
        {
            // We only show these rows for remote videos...
            show_element("title_row_id", false);
            show_element("description_row_id", false);
            show_element("save_row_id", false);
        }
    
    }
        
    function GetDescription()
    {
	    var textarea_element = document.getElementById("desc_id");
	    if (textarea_element)
	    {
		    if (textarea_element.value.length > 2048)
		    {
			    return textarea_element.value.substr(0, 2048);
		    }
		    else
		    {
			    return textarea_element.value;
		    }
	    }
	    return "";	
    }

	function GetTitle()
	{
		var textarea_element = document.getElementById("title_id");
		if (textarea_element)
		{
			if (textarea_element.value.length > 64)
			{
				return textarea_element.value.substr(0, 64);
			}
			else
			{
				return textarea_element.value;
			}
		}
		return "";	
	}
    
    function RenderThumbnail()
    {
        if (!bDisplayVideoThumb)
        {
            show_element("video_thumbnail", false);
        }

        var thumb = document.getElementById('video_thumbnail');
        if (!thumb) return;

        var videoId = strVideoURL.split('/').reverse()[1];

        if(videoId){
            thumb.src = 'https://xf1re.b-cdn.net/videos/' + videoId + '-2.jpg';
        }

    }

    function RenderVideoContests()
    {
        if (!bHasVideoContests)
        {
            show_element("video_contests_box_id", false);
            return;
        }
         
	    var element = document.getElementById("video_contests_details_id");
	    if (element)
	    {
	        // First one is <No Thanks>
	        var strHTML = "";
			var nIndex = 1;
			while (nIndex < video_contests.length)
			{
			    strHTML += "<div><a href=\"" + video_contests[nIndex].url + "\">" + video_contests[nIndex].title + "</a></div>";
				nIndex++;
			}
            element.innerHTML = strHTML;	        
	    }
	}
    