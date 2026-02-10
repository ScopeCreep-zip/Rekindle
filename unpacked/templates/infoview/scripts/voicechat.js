function voicechatupdate(is_microphone, activity_state)
{
    if (is_microphone == 1)
    {
	    //Do nothing anymore
	    /*    
        var micimg = document.getElementById('micimg');
	    if (activity_state == 1)
	    {
	        micimg.style.display='block';
	    }
	    else
	    {
	        micimg.style.display='none';
	    }
	    */
    }
    else
    {
        var speakerimg = document.getElementById('speakerimg');
        if (speakerimg)
        {
            if (activity_state == 1)
            {
                speakerimg.style.visibility = 'visible';
            }
            else
            {
                speakerimg.style.visibility = 'hidden';
            } 
        }
    }
}
