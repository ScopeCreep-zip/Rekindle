
		/*
		** Returns the 'featured' image, otherwise returns the 'boxshot'.
		*/		
		function GetFeaturedImage(images)
		{
			var strImageURL = "";
			for (a = 0; a < images.length; a++)
			{
				if (images[a].getAttribute('type') == "featured")
				{
					strImageURL = images[a].firstChild.nodeValue;
					break;
				}
				else if (images[a].getAttribute('type') == "boxshot")
				{
					strImageURL = images[a].firstChild.nodeValue;
				}
			}
			return strImageURL;
		}

		/*
		** Displays screenshots.
		*/
		function DisplayScreenShots(images)
		{
			var bPrinted = false;
			var bottom_div = document.getElementById('bottom');
			var str = "";
			for (b = 0; b < images.length; b++)
			{
				if (images[b].getAttribute('type') == "screen-shot")
				{
					if (bPrinted == false)
					{
						var strScreenShots = "%screenshots_text%";
						str += "<div class='detail_title'>" + strScreenShots + "</div>";
						bPrinted = true;
					}

					str += "<img class='screenshots' src='" + images[b].firstChild.nodeValue + "'/>";
				}
			}
			bottom_div.innerHTML += str;
		}

		/*
		** Display file description.
		*/		
		function DisplayFileDescription(description)
		{
			var bHasFileAttributeFlag = %has_file_attribute_flag%;
			var bottom_div = document.getElementById('bottom');
			var str = "";
			if (bHasFileAttributeFlag)
			{
				var strFileAttributeTitle = "%file_attribute_title%";
				var strFileAttributeDesc  = "%file_attribute_desc%";
				str += "<div class='detail_title'>" + strFileAttributeTitle + "</div>";
				str += "<div>" + strFileAttributeDesc + "</div>";
				str += "<br>";

				var bFileAttributeIsSkin = %file_attribute_is_skin%;
				if (bFileAttributeIsSkin)
				{
					var vSkins = [%file_attribute_skin_vector%];
					for (i = 0; i < vSkins.length; i++)
					{
						var strSwitch = "Switch to the " + vSkins[i] + " skin";
						str += "<div><span class='fakelink' action='skin' skin='" + vSkins[i] + "'>" + strSwitch + "</span></div>";
					}
					
					if (vSkins.length)
						str += "<br>";				
				}
			}

			var strDescTitle = "%description_text%";
			str += "<div class='detail_title'>" + strDescTitle + "</div>";
			str += "<div>" + description + "</div>";
			str += "<br>";

			bottom_div.innerHTML += str;
		}
		
		/*
		** Debug function.
		*/
		function DebugImageData()
		{
			for (i = 0; i < images.length; i++)
			{
				document.write("<div>" + i + ". " + images[i].type + ", " + images[i].url + "</div>");
			}
		}


		/*
		** AJAX function to request file data from server.
		*/
		function requestData()
		{
			var bottom_div = document.getElementById('bottom');
			bottom_div.innerHTML = "";
			
			var nFileId = %selected_file_id%;
			if (!nFileId)
			{
				bottom_div.innerHTML = "<p>download.js requestData() - Unknown file id</p>";
				return;
			}
						
			AjaxRequest.get(
				{
					'url':"%scripting_host%/v4/client/filesinfo.php?type=file&id=" + nFileId,
					'timeout':2000,
					'onSuccess':
						function (req)
						{
							var description = "";
							var description_elem = req.responseXML.getElementsByTagName('description');
							if (description_elem.length)
							{
								description = description_elem[0].firstChild.nodeValue;
							}
							var images = req.responseXML.getElementsByTagName('image');
							var strImgSrc = GetFeaturedImage(images);
							if (strImgSrc.length)
							{
								var img = document.getElementById('image_holder');
								var strFileURL = "%xfire_files_url%" + "/" + nFileId;
								img.innerHTML = "<a href='" + strFileURL + "' target='_blank'><img class='featured_image' src='" + strImgSrc + "'/></a><br>";
								img.style.display = "";
							}
							DisplayFileDescription(description);
							DisplayScreenShots(images);
							var newEvt = document.createEventObject();
							var bRet = document.getElementById('request_finished_id').fireEvent("ondataavailable", newEvt);
						},
					'onError':
						function (req)
						{
							bottom_div.innerHTML =  "%text_error_fetch%, <b style='cursor:pointer;text-decoration:underline' onclick='requestData();'>%text_error_retry%</b>";
						},
					'onTimeout':
						function (req)
						{
							bottom_div.innerHTML = "%text_error_timeout%, <b style='cursor:pointer;text-decoration:underline' onclick='requestData();'>%text_error_retry%</b>";
						}
				}
			);
		}
