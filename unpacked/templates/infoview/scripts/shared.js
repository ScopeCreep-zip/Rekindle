		/////////////////////////////////////////////////////////////////////
		// shared.js
		// - Common javascript functions
		/////////////////////////////////////////////////////////////////////
		
		function show_element(strId, bShow) 
		{ 
			var element = document.getElementById(strId);
			if (element)
			{
				if (bShow)
					element.style.display = "block";
				else
					element.style.display = "none";
			}
		} 

		//////////////////////////////////////////////////////////////////////////////////////
		// RebuildEventSinks()
		// - Any time new elements are dynamically added or removed, we need to inform the 
		//   client app.  We do this by firing off an ONDATAAVAILABLE event which causes
		//   the xfire client to rebuild its internal HTML event sinks.
		//////////////////////////////////////////////////////////////////////////////////////
		function RebuildEventSinks()
		{
			var newEvt = document.createEventObject();
			var element = document.getElementById('fire_event_id');
			if (element)
				element.fireEvent("ondataavailable", newEvt);
		}		

		//////////////////////////////////////////////////////////////////////////////////////
		// GetCoordinates()
		// - Retrieve coordinates for any HTML element.
		/////////////////////////////////////////////////////////////////////////////////
		function GetCoordinates(element)
		{
			var coords = { x: 0, y: 0, width: element.offsetWidth, height: element.offsetHeight};
			while (element)
			{
				coords.x += element.offsetLeft;
				coords.y += element.offsetTop;
				element = element.offsetParent;
			}
			return coords;
		}
