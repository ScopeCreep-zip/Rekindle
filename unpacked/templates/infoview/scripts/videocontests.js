	/////////////////////////////////////////////////////////////////////
	// videocontests.js
	// - Common video contest related javascript functions
	/////////////////////////////////////////////////////////////////////
		
    var bHasVideoContests = %has_video_contests%;
	var video_contests = [ %video_contests% ];

	function render_video_contests()
	{
		if (!bHasVideoContests)
		{
			show_element("video_contests_box", false);
			return;
		}

		var element = document.getElementById("video_contests_details");
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
    
    