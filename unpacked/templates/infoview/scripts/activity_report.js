function request_activity_report_success(req)
{
	var report_details = document.getElementById('activity_report_details');
	if (report_details)
	{
        show_element("activity_report_box", 1);

		report_details.innerHTML = req.responseText;

		RebuildEventSinks();
	}
}

function request_activity_report_error(req)
{
	var report_details = document.getElementById('activity_report_details');
	if (report_details)
	{

		report_details.innerHTML = "<p>%text_error_activity_report_error%</p>";
		
		RebuildEventSinks();
	}
}

function request_activity_report_timeout(req)
{
	var report_details = document.getElementById('activity_report_details');
	if (report_details)
	{
		report_details.innerHTML = "<p>%text_error_activity_report_timeout%</p>";

		// Any time new elements are dynamically added/removed, we need to inform the client app.
		// Fire off an event which will tell the client to rebuild the html event sinks.
		RebuildEventSinks();
	}
}

function request_activity_report()
{
	var activity_report_request_url = "%activity_report_request_url%";

    AjaxRequest.get(
        {
            'generateUniqueUrl' : true,
            'url'               : activity_report_request_url,
            'timeout'           : 5000,
            'onSuccess'         : function (req) { request_activity_report_success(req); },
            'onError'           : function (req) { request_activity_report_error(req); },
            'onTimeout'         : function (req) { request_activity_report_timeout(req); }
        }
    );	  
}

// sets the img width= tag to enable scaling ONLY when desired size is smaller (downscampling is okay).
// Otherwise, leaves it alone to avoid upsampling (too blocky)
function SetImageSizeConditional( image, maxSize, autoHCenter )
{
    // get image width
    if( typeof( image.naturalWidth ) != "undefined" && false)
        width = parseInt( image.naturalWidth );
    else 
        width = parseInt( image.width );

    if( typeof( image.naturalHeight ) != "undefined" )
        height = parseInt( image.naturalHeight );
    else 
        height = parseInt( image.height );

    if( width == 0 || height == 0 )
    {
        //alert( "SetImageWidthCondition( " + image.src + " ) w" + width + " h" + height + " c" + image.complete );
        return;
    }

    // scale if image exceeds max in either direction
    greater = Math.max( width, height );
    if( greater > maxSize )
    {
        // trigger downsampling
        scale = maxSize / greater;
        width = Math.round( width * scale );
        height = Math.round( height * scale );
        image.width = "" + width;
        image.height = "" + height;
        //alert( "s" + scale + " nw" + width + " nh" + height +  " w" + image.width + " h" + image.height + " max" + maxSize );
    }

    // if width (either original, or re-computed) is under the limit now
    if( width < maxSize && autoHCenter == 1 )
    {
        // auto-hcenter within target size, since we're undersized
        margin = (maxSize - width ) / 2;
        image.style.marginLeft = "" + margin  + "px";
    }
}

 

